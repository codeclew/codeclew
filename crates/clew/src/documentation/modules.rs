//! Documentation-facing selection over the existing language producers.
use super::{digest, invalid, model::*, store::Repository};
use crate::{
    analysis_modules, error::ClewError, kotlin_engine::KotlinSemanticEngine,
    runtime::RuntimeAuthority,
};
use clap::Subcommand;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Configuration {
    pub schema: String,
    #[serde(default)]
    pub semantic: Option<SemanticModule>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SemanticModule {
    pub module: String,
    pub enabled: bool,
    #[serde(default)]
    pub profile: String,
    #[serde(default)]
    pub compilation: String,
}
#[derive(Debug, Subcommand)]
pub enum Command {
    List {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        service: Option<String>,
    },
    Show {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        id: String,
        #[arg(long)]
        service: Option<String>,
    },
}
/// The explicit module object takes precedence only when no legacy semantic
/// selection is present. Ambiguous declarations are rejected at registration.
pub(super) fn semantic(service: &Service) -> Option<SemanticConfig> {
    if let Some(config) = &service.modules {
        return config
            .semantic
            .as_ref()
            .filter(|m| m.enabled)
            .map(|m| SemanticConfig {
                profile: m.profile.clone(),
                compilation: m.compilation.clone(),
            });
    }
    service.source.as_ref().and_then(|s| s.semantic.clone())
}
pub(super) fn validate(service: &Service) -> Result<(), ClewError> {
    let Some(config) = &service.modules else {
        return Ok(());
    };
    if config.schema != "codeclew-documentation-modules/1.0"
        || service.profile != "source-syntax"
        || service
            .source
            .as_ref()
            .is_some_and(|s| s.semantic.is_some())
    {
        return Err(invalid(
            "explicit documentation modules require source-syntax and cannot also use legacy source.semantic",
        ));
    }
    if let Some(module) = &config.semantic {
        if !matches!(
            (service.language.as_str(), module.module.as_str()),
            ("java", "javac") | ("kotlin", "kotlin-k2")
        ) {
            return Err(invalid(
                "documentation semantic module is not applicable to the service language",
            ));
        }
        if module.enabled {
            let mut provider = service.clone();
            provider.modules = None;
            provider.source = None;
            provider.profile = module.profile.clone();
            provider.compilation = module.compilation.clone();
            super::store::validate_service(&provider)?;
        } else if !module.profile.is_empty() || !module.compilation.is_empty() {
            return Err(invalid(
                "disabled semantic module must not carry executable project selection",
            ));
        }
    }
    Ok(())
}
pub(super) fn catalog() -> Result<Vec<Value>, ClewError> {
    let runtime = RuntimeAuthority::from_environment()?;
    let registered = runtime
        .as_ref()
        .map(analysis_modules::registered)
        .unwrap_or_default();
    let common = |id: &str,
                  languages: Value,
                  authority: &str,
                  input: Value,
                  output: Value,
                  implementation: String| json!({"schema":"codeclew-documentation-module-capability/1.0","id":id,"languages":languages,"authority":authority,"inputSchemas":input,"outputSchemas":output,"implementationDigest":implementation,"selection":"OPERATOR_DOCUMENTATION_CATALOG_ONLY","execution":"EXISTING_PRODUCER_ADMISSION_AND_CLEANUP"});
    let mut source = common(
        "source-syntax",
        json!(["java", "kotlin", "python"]),
        "SYNTAX_ONLY",
        json!([crate::repository_snapshot::SNAPSHOT_SCHEMA]),
        json!([
            "codeclew-documentation-service-evidence/1.0",
            clew_facts::SOURCE_ANNOTATION_SCHEMA
        ]),
        digest(&(
            crate::canonical::hash_bytes(include_bytes!("syntax.rs")),
            crate::canonical::hash_bytes(include_bytes!("source_annotations.rs")),
            crate::canonical::hash_bytes(include_bytes!("../../../clew-facts/src/lib.rs")),
            crate::canonical::hash_bytes(include_bytes!("../../../../Cargo.lock")),
        ))?,
    );
    source["availability"] = json!("BUILT_IN_NO_BUILD_TOOLS");
    source["projectCompatibility"] = json!("DECLARED_DIALECT_WITH_SYNTAX_GAPS");
    let mut java = common(
        "javac",
        json!(["java"]),
        "COMPILER_BACKED_JDK",
        json!([crate::java_project_model::JAVA_MODEL_SCHEMA]),
        json!([
            "codeclew-java-compiler-fact/1.0",
            clew_facts::JVM_ANNOTATION_SCHEMA
        ]),
        crate::canonical::hash_bytes(include_bytes!("../java_analyzer.java")),
    );
    java["producer"] = registered
        .iter()
        .find(|m| m.id == "java17")
        .map(serde_json::to_value)
        .transpose()
        .map_err(super::io_error)?
        .unwrap_or(Value::Null);
    java["availability"] = json!("PROJECT_ADMISSION_REQUIRED");
    java["projectJavaMinimum"] = json!(analysis_modules::JAVA_MIN_MAJOR);
    java["analyzer"] = json!("PROJECT_NATIVE_JAVAC");
    let engines: Vec<_> = registered
        .iter()
        .filter(|m| m.language == Some("kotlin"))
        .collect();
    let mut kotlin = common(
        "kotlin-k2",
        json!(["kotlin"]),
        "K2_RESOLVED",
        json!(["kotlin-semantic-input-manifest/0.1"]),
        json!([
            "declaration-descriptor/0.1",
            clew_facts::JVM_ANNOTATION_SCHEMA
        ]),
        digest(&engines)?,
    );
    kotlin["availability"] = json!(if engines.is_empty() {
        "WORKER_NOT_INSTALLED"
    } else {
        "PROJECT_ADMISSION_REQUIRED"
    });
    kotlin["producers"] = json!(engines);
    kotlin["knownAnalyzers"] = json!(
        KotlinSemanticEngine::all_known()
            .iter()
            .map(|e| e.authority())
            .collect::<Vec<_>>()
    );
    kotlin["workerJavaMajor"] =
        json!(analysis_modules::kotlin_worker_jvm(KotlinSemanticEngine::Kotlin24).java_major);
    kotlin["projectCompatibility"] = json!({"minimumKotlin":"1.9","maximumKotlinLine":"2.4","gate":"EXISTING_PROJECT_SEMANTICS_OPTIONS_AND_PLUGIN_ABI_ADMISSION","workerJdkIsNotProjectTarget":true});
    let mut spring = common(
        clew_framework_spring::MODULE_ID,
        json!(["java", "kotlin"]),
        "DERIVED_FROM_EXPLICIT_INPUT_AUTHORITY",
        json!([
            clew_facts::JVM_ANNOTATION_SCHEMA,
            clew_facts::SOURCE_ANNOTATION_SCHEMA
        ]),
        json!(["spring-entrypoints/0.2"]),
        clew_framework_spring::implementation_digest(),
    );
    spring["availability"] = json!("BUILT_IN_SOURCE_OR_SEALED_FACTS");
    spring["projectCompatibility"] = json!("QUALIFIED_FRAMEWORK_RULES_ONLY");
    let mut openapi = common(
        "openapi",
        json!(["java", "kotlin", "python"]),
        "DECLARED_OPENAPI",
        json!(["openapi/3.0.0", "openapi/3.0.3"]),
        json!([super::contracts::SCHEMA]),
        crate::canonical::hash_bytes(include_bytes!("contracts.rs")),
    );
    openapi["availability"] = json!("BUILT_IN_NO_BUILD_TOOLS");
    openapi["testedVersions"] = json!(super::contracts::TESTED_VERSIONS);
    openapi["limitations"] = json!([
        "EXPLICIT_COMMITTED_FILES_ONLY",
        "NO_IMPLICIT_NETWORK",
        "DECLARATIONS_NOT_RUNTIME_ENFORCEMENT",
        "CALLBACKS_RETAINED_NOT_SOURCE_MAPPED",
        "NO_SCHEMA_INSTANCE_VALIDATION"
    ]);
    Ok(vec![source, java, kotlin, spring, openapi])
}
fn applicable(value: &Value, service: &Service) -> bool {
    value["languages"]
        .as_array()
        .is_some_and(|l| l.iter().any(|s| s == &service.language))
}
pub fn run(command: Command) -> Result<Value, ClewError> {
    let (root, service, id) = match command {
        Command::List { root, service } => (root, service, None),
        Command::Show { root, id, service } => (root, service, Some(id)),
    };
    let repo = Repository::open(&root)?;
    let services = repo.services()?;
    let selected = service
        .as_ref()
        .map(|s| services.get(s).ok_or_else(|| invalid("unknown service")))
        .transpose()?;
    let mut records = catalog()?;
    for record in &mut records {
        if let Some(service) = selected {
            record["applicable"] = json!(applicable(record, service));
            record["configured"] = json!(match record["id"].as_str() {
                Some("source-syntax") => service.profile == "source-syntax",
                Some("javac") =>
                    service.language == "java"
                        && (semantic(service).is_some() || service.profile != "source-syntax"),
                Some("kotlin-k2") =>
                    service.language == "kotlin"
                        && (semantic(service).is_some() || service.profile != "source-syntax"),
                Some("openapi") => !service.contract_files.is_empty(),
                Some("spring") => matches!(service.language.as_str(), "java" | "kotlin"),
                _ => false,
            });
        }
    }
    if let Some(id) = id {
        return Ok(
            json!({"schema":"codeclew-documentation-module-result/1.0","service":service,"record":records.into_iter().find(|r|r["id"]==id).ok_or_else(||invalid("unknown documentation module"))?}),
        );
    }
    Ok(
        json!({"schema":"codeclew-documentation-module-list/1.0","service":service,"records":records}),
    )
}
/// Provider/rule implementation and availability participate in invalidation even
/// when source bytes do not change. No registration path executes repository code.
pub(super) fn attach(service: &Service, evidence: &mut ServiceEvidence) -> Result<(), ClewError> {
    let selected = semantic(service);
    let records: Vec<_> = catalog()?
        .into_iter()
        .filter(|r| match r["id"].as_str() {
            Some("source-syntax") => service.profile == "source-syntax",
            Some("javac") => {
                service.language == "java"
                    && (selected.is_some() || service.profile != "source-syntax")
            }
            Some("kotlin-k2") => {
                service.language == "kotlin"
                    && (selected.is_some() || service.profile != "source-syntax")
            }
            Some("openapi") => !service.contract_files.is_empty(),
            _ => matches!(service.language.as_str(), "java" | "kotlin"),
        })
        .collect();
    let normalized = json!({"schema":"codeclew-documentation-module-influence/1.0","configuration":service.modules,"legacySemantic":service.source.as_ref().and_then(|s|s.semantic.as_ref()),"modules":records,"adapterProtocol":crate::adapter_v2::ADAPTER_PROTOCOL,"derivationDigest":digest(&(crate::canonical::hash_bytes(include_bytes!("analysis.rs")),crate::canonical::hash_bytes(include_bytes!("modules.rs"))))?});
    if let Some(scope) = evidence
        .observations
        .values_mut()
        .find(|o| o.kind == "SOURCE_SCOPE")
    {
        scope.normalized["modules"] = normalized;
        scope.digest = digest(&scope.normalized)?;
    } else {
        let id =
            super::analysis::dependency_id(&service.id, "MODULE_SCOPE", "documentation-modules")?;
        evidence.observations.insert(
            id.clone(),
            Observation {
                id,
                symbol: "documentation-modules".into(),
                kind: "MODULE_SCOPE".into(),
                service: service.id.clone(),
                digest: digest(&normalized)?,
                normalized,
                source_ids: vec![],
            },
        );
    }
    super::analysis::verify_evidence(evidence)
}
