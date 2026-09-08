//! Built-in module registration. Installation and project admission are separate.
use crate::kotlin_engine::KotlinSemanticEngine;
use crate::runtime::RuntimeAuthority;
use serde::Serialize;

pub const JAVA_MIN_MAJOR: u16 = 17;

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerJvmRequirement {
    pub java_major: u16,
}

pub fn kotlin_worker_jvm(_engine: KotlinSemanticEngine) -> WorkerJvmRequirement {
    WorkerJvmRequirement { java_major: 21 }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisModule {
    pub schema: &'static str,
    pub id: String,
    pub kind: &'static str,
    pub version: &'static str,
    pub implementation_digest: String,
    pub language: Option<&'static str>,
    pub compiler: Option<String>,
    pub project_java_minimum: Option<u16>,
    pub worker_jvm: Option<WorkerJvmRequirement>,
    pub input_schemas: Vec<&'static str>,
    pub output_schemas: Vec<&'static str>,
    pub operation_scope: &'static str,
}

pub fn registered(runtime: &RuntimeAuthority) -> Vec<AnalysisModule> {
    let mut modules = vec![AnalysisModule {
        schema: "codeclew-analysis-module/1.0",
        id: "java17".into(),
        kind: "LANGUAGE",
        version: env!("CARGO_PKG_VERSION"),
        implementation_digest: crate::canonical::hash_bytes(include_bytes!("java_analyzer.java")),
        language: Some("java"),
        compiler: Some("PROJECT_NATIVE_JAVAC".into()),
        project_java_minimum: Some(JAVA_MIN_MAJOR),
        worker_jvm: None,
        input_schemas: vec![crate::java_project_model::JAVA_MODEL_SCHEMA],
        output_schemas: vec![
            "codeclew-java-compiler-fact/1.0",
            clew_facts::JVM_ANNOTATION_SCHEMA,
        ],
        operation_scope: "READ_ONLY_PROJECT_ADMISSION_REQUIRED",
    }];
    for engine in KotlinSemanticEngine::all_known() {
        if let Some(worker) = runtime.workers.get(engine.runtime_name()) {
            modules.push(AnalysisModule {
                schema: "codeclew-analysis-module/1.0",
                id: engine.runtime_name().into(),
                kind: "LANGUAGE",
                version: env!("CARGO_PKG_VERSION"),
                implementation_digest: worker.tree_hash.clone(),
                language: Some("kotlin"),
                compiler: Some(engine.analyzer_compiler_version().into()),
                project_java_minimum: None,
                worker_jvm: Some(kotlin_worker_jvm(engine)),
                input_schemas: vec!["kotlin-semantic-input-manifest/0.1"],
                output_schemas: vec![
                    "declaration-descriptor/0.1",
                    clew_facts::JVM_ANNOTATION_SCHEMA,
                ],
                operation_scope: "PROJECT_AND_OPERATION_ADMISSION_REQUIRED",
            });
        }
    }
    modules.push(AnalysisModule {
        schema: "codeclew-analysis-module/1.0",
        id: clew_framework_spring::MODULE_ID.into(),
        kind: "FRAMEWORK",
        version: env!("CARGO_PKG_VERSION"),
        implementation_digest: clew_framework_spring::implementation_digest(),
        language: None,
        compiler: None,
        project_java_minimum: None,
        worker_jvm: None,
        input_schemas: vec![clew_facts::JVM_ANNOTATION_SCHEMA],
        output_schemas: vec!["spring-entrypoints/0.2"],
        operation_scope: "DERIVATION_OVER_SEALED_FACTS",
    });
    modules
}
