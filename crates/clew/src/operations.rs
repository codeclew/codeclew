use crate::canonical;
use crate::error::{ClewError, ErrorCode};
use crate::repository_snapshot::isolated_git_command;
use crate::runtime::RuntimeAuthority;
use crate::state::StateAuthority;
use serde_json::{Value, json};
use std::ffi::CString;
use std::fs;
use std::path::Path;
use std::process::Stdio;

#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

const SUPPORT_MATRIX_BYTES: &[u8] = include_bytes!("../support-matrix.json");
const SUPPORT_MATRIX_SCHEMA: &str = "codeclew-support-matrix/1.0";
const SUPPORT_SUMMARY_SCHEMA: &str = "codeclew-support-summary/1.0";
const COLD_BUILD_MINIMUM_FREE_BYTES: u64 = 6 * 1024 * 1024 * 1024;

pub fn support_matrix() -> Result<Value, ClewError> {
    let matrix: Value = serde_json::from_slice(SUPPORT_MATRIX_BYTES)
        .map_err(|_| invalid("embedded support matrix is invalid"))?;
    let canonical_source = SUPPORT_MATRIX_BYTES
        .strip_suffix(b"\n")
        .unwrap_or(SUPPORT_MATRIX_BYTES);
    if matrix.get("schema").and_then(Value::as_str) != Some(SUPPORT_MATRIX_SCHEMA)
        || matrix.get("status").and_then(Value::as_str) != Some("PILOT_READY")
        || !matrix.get("profiles").is_some_and(Value::is_array)
        || canonical::bytes(&matrix).map_err(internal)? != canonical_source
    {
        return Err(invalid(
            "embedded support matrix is not canonical or complete",
        ));
    }
    Ok(matrix)
}

pub fn capabilities(runtime: &RuntimeAuthority) -> Result<Value, ClewError> {
    let matrix = support_matrix()?;
    let packaged_workers = runtime
        .workers
        .iter()
        .map(|(runtime_name, worker)| {
            json!({
                "compilerVersion":worker.compiler_version,
                "runtimeName":runtime_name,
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "schema":"codeclew-capabilities/1.0",
        "productVersion":env!("CARGO_PKG_VERSION"),
        "status":"PILOT_READY",
        "runtimeMode":runtime.mode,
        "supportMatrix":matrix,
        "supportMatrixDigest":canonical::hash(&matrix).map_err(internal)?,
        "packagedWorkers":packaged_workers,
        "privacyAssertions":{
            "containsAbsolutePaths":false,
            "containsRepositoryIdentity":false,
            "containsSource":false,
        },
    }))
}

#[derive(Debug, Clone)]
struct DoctorCheck {
    id: &'static str,
    passed: bool,
    required: bool,
    remediation: Option<&'static str>,
}

impl DoctorCheck {
    fn value(&self) -> Value {
        json!({
            "checkId":self.id,
            "required":self.required,
            "status":if self.passed { "PASS" } else { "ACTION_REQUIRED" },
            "remediationId":if self.passed { None } else { self.remediation },
        })
    }
}

pub fn doctor(
    runtime: &RuntimeAuthority,
    repository: Option<&Path>,
    target_ref: Option<&str>,
) -> Result<Value, ClewError> {
    let state = StateAuthority::process_default()?;
    let mut checks = vec![
        check(
            "platform.posix",
            cfg!(unix),
            true,
            "USE_SUPPORTED_POSIX_HOST",
        ),
        check("tool.git", executable_available("git"), true, "INSTALL_GIT"),
        check(
            "tool.python3",
            executable_available("python3"),
            true,
            "INSTALL_PYTHON_3_11",
        ),
        check(
            "tool.java",
            executable_available("java"),
            true,
            "INSTALL_JDK_21",
        ),
        check(
            "tool.rustc",
            executable_available("rustc"),
            true,
            "INSTALL_RUST_1_92",
        ),
        check(
            "tool.cargo",
            executable_available("cargo"),
            true,
            "INSTALL_RUST_1_92",
        ),
        check(
            "state.free-space",
            available_bytes(state.root())
                .is_some_and(|value| value >= COLD_BUILD_MINIMUM_FREE_BYTES),
            true,
            "FREE_6_GIB_ON_STATE_VOLUME",
        ),
        check(
            "runtime.kotlin24",
            runtime
                .workers
                .values()
                .any(|worker| worker.compiler_version == "2.4.10"),
            true,
            "INSTALL_QUALIFIED_RUNTIME",
        ),
        check(
            "runtime.kotlin23",
            runtime
                .workers
                .values()
                .any(|worker| worker.compiler_version == "2.3.0"),
            false,
            "INSTALL_KOTLIN23_PREVIEW_COMPONENT",
        ),
    ];
    if let Some(repository) = repository {
        checks.extend(repository_checks(repository, target_ref));
    }
    let required_passed = checks.iter().all(|row| !row.required || row.passed);
    Ok(json!({
        "schema":"codeclew-doctor/1.0",
        "status":if required_passed { "PASS" } else { "ACTION_REQUIRED" },
        "runtimeMode":runtime.mode,
        "checks":checks.iter().map(DoctorCheck::value).collect::<Vec<_>>(),
        "supportMatrixDigest":canonical::hash(&support_matrix()?).map_err(internal)?,
        "privacyAssertions":{
            "containsAbsolutePaths":false,
            "containsRepositoryIdentity":false,
            "containsSource":false,
        },
    }))
}

fn repository_checks(repository: &Path, target_ref: Option<&str>) -> Vec<DoctorCheck> {
    let Ok(repository) = repository.canonicalize() else {
        return vec![check(
            "repository.available",
            false,
            true,
            "SELECT_EXISTING_REPOSITORY",
        )];
    };
    let git_repository = isolated_git(&repository, &["rev-parse", "--is-inside-work-tree"])
        .is_some_and(|value| value == b"true\n");
    let mut checks = vec![check(
        "repository.git",
        git_repository,
        true,
        "SELECT_GIT_REPOSITORY",
    )];
    if !git_repository {
        return checks;
    }
    let clean = isolated_git(
        &repository,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )
    .is_some_and(|value| value.is_empty());
    checks.push(check(
        "repository.clean",
        clean,
        true,
        "CLEAN_TARGET_WORKTREE",
    ));
    if let Some(target_ref) = target_ref {
        let head = isolated_git(&repository, &["rev-parse", "--verify", "HEAD^{commit}"]);
        let target = isolated_git(
            &repository,
            &["rev-parse", "--verify", &format!("{target_ref}^{{commit}}")],
        );
        checks.push(check(
            "repository.target-ref-at-head",
            head.is_some() && head == target,
            true,
            "CHECKOUT_TARGET_REF_AT_HEAD",
        ));
    }
    let gradle = regular_non_symlink(&repository.join("gradlew"));
    let maven = regular_non_symlink(&repository.join("pom.xml"));
    let python = regular_non_symlink(&repository.join("pyproject.toml"))
        || regular_non_symlink(&repository.join("setup.py"));
    let rust = regular_non_symlink(&repository.join("Cargo.toml"))
        && regular_non_symlink(&repository.join("Cargo.lock"));
    checks.push(check(
        "repository.recognized-project-marker",
        gradle || maven || python || rust,
        false,
        "SELECT_EXPLICIT_LANGUAGE_AND_COMPILATION",
    ));
    checks
}

fn check(id: &'static str, passed: bool, required: bool, remediation: &'static str) -> DoctorCheck {
    DoctorCheck {
        id,
        passed,
        required,
        remediation: Some(remediation),
    }
}

fn isolated_git(repository: &Path, arguments: &[&str]) -> Option<Vec<u8>> {
    let mut command = isolated_git_command(repository);
    let output = command
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    output.status.success().then_some(output.stdout)
}

fn regular_non_symlink(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
}

fn executable_available(name: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|directory| {
        let candidate = directory.join(name);
        fs::metadata(candidate).is_ok_and(|metadata| {
            metadata.is_file() && {
                #[cfg(unix)]
                {
                    metadata.permissions().mode() & 0o111 != 0
                }
                #[cfg(not(unix))]
                {
                    true
                }
            }
        })
    })
}

fn available_bytes(path: &Path) -> Option<u64> {
    fn into_u64<T: Into<u64>>(value: T) -> u64 {
        value.into()
    }

    #[cfg(unix)]
    {
        let path = CString::new(path.as_os_str().as_bytes()).ok()?;
        let mut value = std::mem::MaybeUninit::<libc::statvfs>::uninit();
        if unsafe { libc::statvfs(path.as_ptr(), value.as_mut_ptr()) } != 0 {
            return None;
        }
        let value = unsafe { value.assume_init() };
        Some(into_u64(value.f_bavail).saturating_mul(into_u64(value.f_frsize)))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}

pub fn support_summary(input: &Value) -> Result<Value, ClewError> {
    let schema = input
        .get("schema")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("diagnostic input has no schema"))?;
    let (error, terminal_status, source_stage) = match schema {
        "codeclew-error/2.0" => (
            Some(parse_error_code(input.pointer("/error/code").ok_or_else(
                || invalid("error diagnostic has no typed code"),
            )?)?),
            None,
            "CORE",
        ),
        "codeclew-change-status/1.0" | "codeclew-task-run-status/3.0" => {
            let status = input
                .pointer("/run/status")
                .and_then(Value::as_str)
                .filter(|value| valid_run_status(value))
                .ok_or_else(|| invalid("run diagnostic has no valid status"))?;
            let error = input
                .pointer("/run/failure/code")
                .map(parse_error_code)
                .transpose()?;
            (error, Some(status), "RUN")
        }
        "codeclew-bootstrap-error/1.0" | "codeclew-bootstrap-error/2.0" => {
            (None, Some("BOOTSTRAP_FAILED"), "BOOTSTRAP")
        }
        _ => return Err(invalid("diagnostic input schema is not shareable")),
    };
    let code = error
        .as_ref()
        .map(|value| serde_json::to_value(value).map_err(internal))
        .transpose()?;
    let retryable = error.as_ref().is_some_and(error_code_retryable);
    let remediation_id = error
        .as_ref()
        .map(remediation_for_error)
        .unwrap_or_else(|| remediation_for_status(terminal_status));
    let mut summary = json!({
        "schema":SUPPORT_SUMMARY_SCHEMA,
        "status":"SAFE_TO_SHARE",
        "sourceSchema":schema,
        "sourceStage":source_stage,
        "errorCode":code,
        "retryable":retryable,
        "terminalStatus":terminal_status,
        "remediationId":remediation_id,
        "privacyAssertions":{
            "containsAbsolutePaths":false,
            "containsArguments":false,
            "containsRepositoryContentDigests":false,
            "containsCredentials":false,
            "containsRepositoryIdentity":false,
            "containsSource":false,
            "containsSymbols":false,
        },
    });
    let digest = canonical::hash(&summary).map_err(internal)?;
    summary["summaryDigest"] = Value::String(digest);
    Ok(summary)
}

fn parse_error_code(value: &Value) -> Result<ErrorCode, ClewError> {
    serde_json::from_value(value.clone()).map_err(|_| invalid("diagnostic error code is unknown"))
}

fn valid_run_status(value: &str) -> bool {
    matches!(
        value,
        "CREATED"
            | "PREPARING"
            | "READY_TO_PUBLISH"
            | "READY_TO_PUBLISH_CONDITIONAL"
            | "VALIDATED_CONDITIONAL"
            | "PUBLISHING"
            | "PUBLISHED"
            | "PUBLISHED_CONDITIONAL"
            | "FAILED"
            | "WORKTREE_RECOVERY_REQUIRED"
            | "CANCELLED"
    )
}

fn error_code_retryable(code: &ErrorCode) -> bool {
    matches!(
        code,
        ErrorCode::WorkerCrashed
            | ErrorCode::RefCompareAndSwapFailed
            | ErrorCode::TransactionRecoveryRequired
            | ErrorCode::WorktreeRecoveryRequired
    )
}

fn remediation_for_error(code: &ErrorCode) -> &'static str {
    match code {
        ErrorCode::StaleTarget
        | ErrorCode::StaleRequiresReslice
        | ErrorCode::RefCompareAndSwapFailed => "OPEN_NEW_SESSION",
        ErrorCode::ProjectModelChanged => "USE_MATCHING_RUNTIME_OR_OPEN_NEW_SESSION",
        ErrorCode::TransactionRecoveryRequired => "RESUME_BOUND_RUN",
        ErrorCode::WorktreeRecoveryRequired => "RUN_CHANGE_RECOVER",
        ErrorCode::WorkerCrashed => "RETRY_ONCE_THEN_CAPTURE_INCIDENT",
        ErrorCode::IncompleteSemanticAnalysis => "EXPAND_OR_REVIEW_OBLIGATIONS",
        ErrorCode::ResourceLimit | ErrorCode::SliceBudgetExceeded => "NARROW_REQUEST",
        ErrorCode::CompileFailed | ErrorCode::TestFailed | ErrorCode::NewDiagnostics => {
            "REVIEW_PRIVATE_VALIDATION"
        }
        ErrorCode::UnsupportedKotlinVersion
        | ErrorCode::UnsupportedProjectConfiguration
        | ErrorCode::UnsupportedLanguage => "CHECK_CAPABILITIES",
        ErrorCode::StateCorrupt | ErrorCode::InputMutated => "STOP_AND_PRESERVE_STATE",
        ErrorCode::PreconditionFailed => "REVIEW_PRECONDITION",
        _ => "REVIEW_TYPED_ERROR",
    }
}

fn remediation_for_status(status: Option<&str>) -> &'static str {
    match status {
        Some("BOOTSTRAP_FAILED") => "CHECK_BOOTSTRAP_REQUIREMENTS",
        Some("CREATED" | "PREPARING") => "POLL_RUN_STATUS",
        Some("READY_TO_PUBLISH" | "READY_TO_PUBLISH_CONDITIONAL") => "REVIEW_BEFORE_PUBLISH",
        Some("VALIDATED_CONDITIONAL") => "RESOLVE_BLOCKING_OBLIGATIONS",
        Some("PUBLISHING" | "WORKTREE_RECOVERY_REQUIRED") => "RUN_CHANGE_RECOVER",
        Some("PUBLISHED" | "PUBLISHED_CONDITIONAL" | "CANCELLED") => "NO_ACTION",
        Some("FAILED") => "CAPTURE_TYPED_FAILURE",
        _ => "REVIEW_STATUS",
    }
}

fn invalid(message: &str) -> ClewError {
    ClewError::new(ErrorCode::InvalidInput, message)
}

fn internal(error: impl std::fmt::Display) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_support_matrix_is_canonical_and_keeps_mutation_qualified() {
        let matrix = support_matrix().unwrap();
        let mutable = matrix["profiles"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|profile| profile["mutation"] == true)
            .collect::<Vec<_>>();
        assert_eq!(mutable.len(), 3);
        assert_eq!(mutable[0]["profileId"], "kotlin-2.4.10-gradle-single");
        assert_eq!(mutable[1]["profileId"], "python-syntax");
        assert_eq!(mutable[1]["status"], "PILOT_READY");
        assert_eq!(mutable[2]["profileId"], "rust-syntax");
        assert_eq!(mutable[2]["status"], "PILOT_READY");
        let java = matrix["profiles"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|profile| profile["language"] == "java")
            .collect::<Vec<_>>();
        assert_eq!(java.len(), 2);
        assert!(java.iter().all(|profile| {
            profile["analysisAuthority"] == "COMPILER_BACKED_JDK"
                && profile["compilerVersion"] == "21"
                && profile["mutation"] == false
                && profile["status"] == "READ_ONLY_PREVIEW"
        }));
        let qualified_patch = matrix["profiles"]
            .as_array()
            .unwrap()
            .iter()
            .find(|profile| profile["profileId"] == "kotlin-2.4.0-gradle-single")
            .unwrap();
        assert_eq!(qualified_patch["semanticEngine"], "kotlin-engine-2.4.10");
        assert_eq!(qualified_patch["mutation"], false);
    }

    #[test]
    fn support_summary_drops_private_error_material() {
        let input = json!({
            "schema":"codeclew-error/2.0",
            "error":{
                "code":"WORKER_CRASHED",
                "message":"/private/repository/src/Secret.kt failed",
                "transactionId":"run:private",
                "evidence":["secret source"],
                "relevantAnchorsOrSymbols":["com.private.Secret"],
                "retryable":true,
            },
        });
        let summary = support_summary(&input).unwrap();
        let encoded = canonical::compact(&summary).unwrap();
        assert_eq!(summary["errorCode"], "WORKER_CRASHED");
        assert_eq!(summary["remediationId"], "RETRY_ONCE_THEN_CAPTURE_INCIDENT");
        for forbidden in [
            "/private",
            "Secret.kt",
            "run:private",
            "secret source",
            "com.private",
        ] {
            assert!(!encoded.contains(forbidden));
        }
    }

    #[test]
    fn support_summary_rejects_unknown_input_and_error_code() {
        assert!(support_summary(&json!({"schema":"foreign/1.0"})).is_err());
        assert!(
            support_summary(&json!({
                "schema":"codeclew-error/2.0",
                "error":{"code":"FUTURE_PRIVATE_FAILURE"},
            }))
            .is_err()
        );
    }

    #[test]
    fn support_summary_drops_bootstrap_failure_details() {
        let summary = support_summary(&json!({
            "schema":"codeclew-bootstrap-error/1.0",
            "error":"runtime input /private/codeclew/workers/Secret.kt changed",
        }))
        .unwrap();
        let encoded = canonical::compact(&summary).unwrap();
        assert_eq!(summary["sourceStage"], "BOOTSTRAP");
        assert_eq!(summary["remediationId"], "CHECK_BOOTSTRAP_REQUIREMENTS");
        assert!(!encoded.contains("/private"));
        assert!(!encoded.contains("Secret.kt"));
    }

    #[test]
    fn support_summary_for_status_contains_no_run_identity() {
        let summary = support_summary(&json!({
            "schema":"codeclew-change-status/1.0",
            "run":{
                "runId":"run:private",
                "sessionId":"session:private",
                "status":"READY_TO_PUBLISH_CONDITIONAL",
            },
            "candidate":{"diff":{"patch":"private source"}},
        }))
        .unwrap();
        let encoded = canonical::compact(&summary).unwrap();
        assert_eq!(summary["remediationId"], "REVIEW_BEFORE_PUBLISH");
        assert!(!encoded.contains("run:private"));
        assert!(!encoded.contains("session:private"));
        assert!(!encoded.contains("private source"));
    }
}
