//! Closed, shareable compiler plugin failure metadata. Never project worker text.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Row {
    #[serde(rename = "2.1.21")]
    Kotlin21,
    #[serde(rename = "2.2.21")]
    Kotlin22,
    #[serde(rename = "2.4.10")]
    Kotlin24,
    #[serde(rename = "2.4.20")]
    Kotlin2420,
    #[serde(rename = "OTHER")]
    Other,
    #[serde(rename = "UNAVAILABLE")]
    Unavailable,
}

impl Row {
    fn for_compiler(version: &str) -> Self {
        match version {
            "2.1.21" => Self::Kotlin21,
            "2.2.21" => Self::Kotlin22,
            "2.4.10" => Self::Kotlin24,
            "2.4.20" => Self::Kotlin2420,
            _ => Self::Other,
        }
    }
}

#[derive(Serialize, Deserialize)]
enum Schema {
    #[serde(rename = "codeclew-compiler-plugin-diagnostic/1")]
    V1,
}

#[derive(PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum Check {
    WorkerRow,
    CompilerRow,
    GradleRow,
    JvmTarget,
    NativeAuthority,
    MainCompileTask,
    JdkProvider,
    ArtifactProvider,
    FreeArgsProvider,
    KaptPlugin,
    KaptTaskProvider,
    StubsTaskProvider,
    KaptTaskType,
    StubsTaskType,
    KgpRow,
    CompileNotInGraph,
    CompileDependsOnKapt,
    KaptDependsOnStubs,
    StubsTaskOutcome,
    KaptTaskOutcome,
    NormalPluginOptions,
    ForbiddenFreeArgs,
    OutputRoots,
    GeneratedJavaPresent,
    GeneratedJavaSelected,
    GeneratedKotlinSelected,
    GeneratedClassesSelected,
    NativeTaskIdentities,
    JdkRelease,
    JdkMajor,
    KaptArtifactPresent,
    KaptArtifactDigest,
    KaptArtifactUnique,
    SpringCompileOutcome,
    SpringCompileInputs,
    SpringCompileOutputs,
    SpringCompileIdentity,
    SpringReplayMismatch,
    JavaProducerOutputs,
    QualifiedNativeKapt,
    QualifiedNativeAllOpen,
}

#[derive(PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum Outcome {
    Passed,
    Failed,
    Unavailable,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum FreeArg {
    Xplugin,
    POption,
    PluginPrefix,
}

#[derive(PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum Rejection {
    AnalyzerSerializationUnavailable,
    AnalyzerPluginUnavailable,
    CompilerVersionMismatch,
    KaptNativeAuthorityRequired,
    AllOpenNativeAuthorityRequired,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Native {
    check: Check,
    outcome: Outcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    free_arg_classifier: Option<FreeArg>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Matches {
    worker: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    compiler: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    gradle: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    jvm_target: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    main_compile_task: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    kgp: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    jdk: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    plugin_options_empty: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    generated_java_present: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    generated_java_selected: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    generated_kotlin_selected: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    generated_classes_selected: Option<bool>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Diagnostic {
    schema: Schema,
    native: Native,
    abi_rejection: Rejection,
    requested_compiler_row: Row,
    effective_compiler_row: Row,
    worker_jdk_major: u8,
    matches: Matches,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    gradle_jvm_major: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    project_jdk_major: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    artifact_candidates: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    artifact_digest_matches: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    known_kapt_registrar: Option<bool>,
}

impl Diagnostic {
    fn consistent(&self) -> bool {
        (self.native.outcome == Outcome::Passed)
            == matches!(
                self.native.check,
                Check::QualifiedNativeKapt | Check::QualifiedNativeAllOpen
            )
            && (self.native.free_arg_classifier.is_none()
                || (self.native.check == Check::ForbiddenFreeArgs
                    && self.native.outcome == Outcome::Failed))
            && self.worker_jdk_major > 0
            && self.gradle_jvm_major != Some(0)
            && self.project_jdk_major != Some(0)
            && match (self.artifact_candidates, self.artifact_digest_matches) {
                (Some(count), Some(matches)) => matches <= count,
                (None, None) => true,
                _ => false,
            }
            && self.known_kapt_registrar
                == (self.abi_rejection == Rejection::KaptNativeAuthorityRequired).then_some(true)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Original {
    cli_version: String,
    worker_version: String,
    worker_distribution_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    runtime_key: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Saved {
    diagnostic: Diagnostic,
    original_failure: Original,
}

fn valid_digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hash| {
        hash.len() == 64
            && hash
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

impl Saved {
    fn valid(&self) -> bool {
        let original = &self.original_failure;
        let parts: Vec<_> = original.cli_version.split('.').collect();
        parts.len() == 3
            && parts.iter().all(|part| {
                !part.is_empty() && part.len() <= 5 && part.bytes().all(|b| b.is_ascii_digit())
            })
            && original.worker_version == "0.1.0"
            && valid_digest(&original.worker_distribution_digest)
            && original.runtime_key.as_deref().is_none_or(valid_digest)
            && self.diagnostic.consistent()
    }
}

pub(crate) fn from_worker(
    wire: &[u8],
    compiler: &str,
    worker_version: &str,
    worker_distribution_digest: &str,
    worker_jdk_major: u16,
) -> Option<Value> {
    if wire.is_empty() || wire.len() > 4096 {
        return None;
    }
    let diagnostic: Diagnostic = serde_json::from_slice(wire).ok()?;
    if diagnostic.effective_compiler_row != Row::for_compiler(compiler)
        || u16::from(diagnostic.worker_jdk_major) != worker_jdk_major
    {
        return None;
    }
    let runtime_key = crate::runtime::RuntimeAuthority::diagnostic_metadata()
        .ok()
        .flatten()
        .and_then(|runtime| runtime["runtimeKey"].as_str().map(str::to_owned));
    let saved = Saved {
        diagnostic,
        original_failure: Original {
            cli_version: env!("CARGO_PKG_VERSION").into(),
            worker_version: worker_version.into(),
            worker_distribution_digest: worker_distribution_digest.into(),
            runtime_key,
        },
    };
    saved.valid().then(|| json!(saved))
}

pub(crate) fn saved_summary(value: &Value) -> Option<Value> {
    let saved: Saved = serde_json::from_value(value.clone()).ok()?;
    saved.valid().then(|| json!(saved))
}

pub(crate) fn report(value: &Value) -> Value {
    if value["reason"] != "UNSUPPORTED_COMPILER_PLUGIN_ABI" {
        return Value::Null;
    }
    saved_summary(&value["compilerPluginDiagnostic"])
        .map(|safe| json!({"status":"RECORDED","diagnostic":safe["diagnostic"],"originalFailure":safe["originalFailure"]}))
        .unwrap_or_else(|| json!({"status":"UNAVAILABLE"}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiler_plugin_diagnostic_rejects_private_unknown_stale_and_out_of_order_fields() {
        let wire = json!({
            "schema":"codeclew-compiler-plugin-diagnostic/1",
            "native":{"check":"FORBIDDEN_FREE_ARGS","outcome":"FAILED","freeArgClassifier":"P_OPTION"},
            "abiRejection":"COMPILER_VERSION_MISMATCH",
            "requestedCompilerRow":"2.2.21","effectiveCompilerRow":"2.4.10","workerJdkMajor":21,
            "matches":{"worker":true,"compiler":true,"gradle":true,"jvmTarget":true}
        });
        let digest = format!("sha256:{}", "a".repeat(64));
        let parse = |wire: &Value| {
            from_worker(
                &serde_json::to_vec(wire).unwrap(),
                "2.4.10",
                "0.1.0",
                &digest,
                21,
            )
        };
        let saved = parse(&wire).unwrap();
        assert_eq!(saved["diagnostic"], wire);
        let mut private = wire.clone();
        private["native"]["message"] = json!("/private/path/token=secret");
        assert!(parse(&private).is_none());
        let mut wrong_row = wire.clone();
        wrong_row["effectiveCompilerRow"] = json!("2.1.21");
        assert!(parse(&wrong_row).is_none());
        let mut wrong_schema = wire.clone();
        wrong_schema["schema"] = json!("codeclew-compiler-plugin-diagnostic/2");
        assert!(parse(&wrong_schema).is_none());
        let mut invented_registrar = wire;
        invented_registrar["knownKaptRegistrar"] = json!(true);
        assert!(parse(&invented_registrar).is_none());
    }
}
