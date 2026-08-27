use crate::canonical;
use crate::error::{ClewError, ErrorCode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::Path;

pub const KOTLIN_ADAPTER_CONTRACT_ID: &str = "kotlin-semantic-facts";
pub const KOTLIN_ENGINE_AUTHORITY_SCHEMA: &str = "codeclew-kotlin-engine-authority/1.0";
pub const KOTLIN_PROJECT_SEMANTICS_SCHEMA: &str = "codeclew-kotlin-project-semantics/1.0";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum KotlinSemanticEngine {
    Kotlin21,
    Kotlin23,
    Kotlin24,
}

impl KotlinSemanticEngine {
    pub const fn packaged_by_preference() -> [Self; 2] {
        [Self::Kotlin24, Self::Kotlin23]
    }

    pub const fn all_known() -> [Self; 3] {
        [Self::Kotlin24, Self::Kotlin23, Self::Kotlin21]
    }

    pub const fn engine_id(self) -> &'static str {
        match self {
            Self::Kotlin21 => "kotlin-engine-2.1.21",
            Self::Kotlin23 => "kotlin-engine-2.3.0",
            Self::Kotlin24 => "kotlin-engine-2.4.10",
        }
    }

    pub const fn runtime_name(self) -> &'static str {
        match self {
            Self::Kotlin21 => "kotlin21",
            Self::Kotlin23 => "kotlin23",
            Self::Kotlin24 => "kotlin24",
        }
    }

    pub const fn analyzer_compiler_version(self) -> &'static str {
        match self {
            Self::Kotlin21 => "2.1.21",
            Self::Kotlin23 => "2.3.0",
            Self::Kotlin24 => "2.4.10",
        }
    }

    pub const fn fir_api_row(self) -> &'static str {
        match self {
            Self::Kotlin21 => "fir-internal-2.1.21",
            Self::Kotlin23 => "fir-internal-2.3.0",
            Self::Kotlin24 => "fir-internal-2.4.10",
        }
    }

    pub const fn facts_extractor_identity(self) -> &'static str {
        match self {
            Self::Kotlin21 => "codeclew-kotlin-facts-2.1",
            Self::Kotlin23 => "codeclew-kotlin-facts-2.3",
            Self::Kotlin24 => "codeclew-kotlin-facts-2.4",
        }
    }

    pub const fn bta_implementation(self) -> &'static str {
        match self {
            Self::Kotlin21 => "kotlin-bta-2.1.21",
            Self::Kotlin23 => "none",
            Self::Kotlin24 => "kotlin-bta-2.4.10",
        }
    }

    pub const fn discovery_bit(self) -> u8 {
        match self {
            Self::Kotlin21 => 1 << 2,
            Self::Kotlin23 => 1 << 0,
            Self::Kotlin24 => 1 << 1,
        }
    }

    pub fn authority(self) -> KotlinEngineCapabilities {
        KotlinEngineCapabilities {
            schema: KOTLIN_ENGINE_AUTHORITY_SCHEMA.into(),
            engine_id: self.engine_id().into(),
            runtime_name: self.runtime_name().into(),
            analyzer_compiler_version: self.analyzer_compiler_version().into(),
            fir_api_row: self.fir_api_row().into(),
            facts_extractor_identity: self.facts_extractor_identity().into(),
            bta_implementation: self.bta_implementation().into(),
        }
    }

    pub fn from_analyzer_compiler_version(version: &str) -> Result<Self, ClewError> {
        Self::all_known()
            .into_iter()
            .find(|engine| engine.analyzer_compiler_version() == version)
            .ok_or_else(|| unsupported("semantic engine compiler version is not packaged"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KotlinEngineCapabilities {
    pub schema: String,
    pub engine_id: String,
    pub runtime_name: String,
    pub analyzer_compiler_version: String,
    pub fir_api_row: String,
    pub facts_extractor_identity: String,
    pub bta_implementation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum KotlinCompilerPluginKind {
    KotlinSerialization,
    KotlinScripting,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KotlinCompilerPluginSemantics {
    pub artifact_name: String,
    pub kind: KotlinCompilerPluginKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KotlinProjectSemantics {
    pub schema: String,
    pub project_compiler_version: String,
    pub compiler_version_authority: String,
    pub language_version: Option<String>,
    pub api_version: Option<String>,
    pub jvm_target: Option<String>,
    pub compiler_plugins: Vec<KotlinCompilerPluginSemantics>,
    pub unstable_compiler_options: Vec<String>,
}

impl KotlinProjectSemantics {
    pub fn from_project_model(model: &Value) -> Result<Self, ClewError> {
        let project_compiler_version = model
            .get("declaredCompilerVersion")
            .or_else(|| model.get("projectCompilerVersion"))
            .or_else(|| model.get("compilerVersion"))
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| unsupported("project Kotlin compiler version is unavailable"))?
            .to_owned();
        let compiler_version_authority = model
            .pointer("/projectCompilerAuthority/source")
            .or_else(|| model.get("compilerVersionAuthority"))
            .and_then(Value::as_str)
            .unwrap_or("LEGACY_MODEL_FIELD")
            .to_owned();
        let string = |field: &str| {
            model
                .get(field)
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        };
        let mut compiler_plugins = model
            .get("compilerPlugins")
            .or_else(|| model.get("requestedCompilerPlugins"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(|raw| {
                let identity = raw
                    .rsplit_once(":sha256:")
                    .map_or(raw, |(identity, _)| identity);
                let locator = identity
                    .strip_prefix("artifact:")
                    .or_else(|| identity.strip_prefix("repo:"))
                    .unwrap_or(identity);
                let artifact_name = Path::new(locator)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(locator)
                    .to_owned();
                let kind = if artifact_name
                    .starts_with("kotlin-serialization-compiler-plugin-embeddable-")
                {
                    KotlinCompilerPluginKind::KotlinSerialization
                } else if artifact_name.starts_with("kotlin-scripting-compiler-embeddable-") {
                    KotlinCompilerPluginKind::KotlinScripting
                } else {
                    KotlinCompilerPluginKind::Unknown
                };
                KotlinCompilerPluginSemantics {
                    artifact_name,
                    kind,
                }
            })
            .collect::<Vec<_>>();
        compiler_plugins.sort_by(|left, right| left.artifact_name.cmp(&right.artifact_name));
        compiler_plugins.dedup();
        let mut unstable_compiler_options = model
            .get("freeCompilerArguments")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter(|argument| argument.starts_with("-X"))
            .map(str::to_owned)
            .collect::<Vec<_>>();
        unstable_compiler_options.sort();
        unstable_compiler_options.dedup();
        Ok(Self {
            schema: KOTLIN_PROJECT_SEMANTICS_SCHEMA.into(),
            project_compiler_version,
            compiler_version_authority,
            language_version: string("languageVersion"),
            api_version: string("apiVersion"),
            jvm_target: string("jvmTarget"),
            compiler_plugins,
            unstable_compiler_options,
        })
    }

    pub fn authority_digest(&self, engine: KotlinSemanticEngine) -> Result<String, ClewError> {
        canonical::hash(&json!({
            "schema":"codeclew-kotlin-semantic-authority/1.0",
            "project":self,
            "engine":engine.authority(),
        }))
        .map_err(internal)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompatibilityKind {
    ExactCompilerAbi,
    QualifiedPatchLine,
    QualificationCandidate,
    ExperimentalCandidate,
}

impl CompatibilityKind {
    const fn is_cross_engine(self) -> bool {
        !matches!(self, Self::ExactCompilerAbi)
    }
}

#[derive(Debug, Clone, Copy)]
struct QualifiedCompatibility {
    project_compiler_version: &'static str,
    language_version: Option<&'static str>,
    api_version: Option<&'static str>,
    engine: KotlinSemanticEngine,
    kind: CompatibilityKind,
    default_route: bool,
    allow_serialization_rebind: bool,
}

const QUALIFIED_COMPATIBILITY: &[QualifiedCompatibility] = &[
    QualifiedCompatibility {
        project_compiler_version: "2.3.0",
        language_version: None,
        api_version: None,
        engine: KotlinSemanticEngine::Kotlin23,
        kind: CompatibilityKind::ExactCompilerAbi,
        default_route: true,
        allow_serialization_rebind: true,
    },
    QualifiedCompatibility {
        project_compiler_version: "2.4.10",
        language_version: None,
        api_version: None,
        engine: KotlinSemanticEngine::Kotlin24,
        kind: CompatibilityKind::ExactCompilerAbi,
        default_route: true,
        allow_serialization_rebind: true,
    },
    QualifiedCompatibility {
        project_compiler_version: "2.4.0",
        language_version: None,
        api_version: None,
        engine: KotlinSemanticEngine::Kotlin24,
        kind: CompatibilityKind::QualifiedPatchLine,
        default_route: true,
        allow_serialization_rebind: true,
    },
    QualifiedCompatibility {
        project_compiler_version: "2.3.0",
        language_version: None,
        api_version: None,
        engine: KotlinSemanticEngine::Kotlin24,
        kind: CompatibilityKind::QualificationCandidate,
        default_route: false,
        allow_serialization_rebind: true,
    },
    QualifiedCompatibility {
        project_compiler_version: "2.1.21",
        language_version: None,
        api_version: None,
        engine: KotlinSemanticEngine::Kotlin24,
        kind: CompatibilityKind::ExperimentalCandidate,
        default_route: false,
        allow_serialization_rebind: true,
    },
];

#[derive(Debug, Default, Clone, Copy)]
pub struct KotlinEngineRegistry;

impl KotlinEngineRegistry {
    pub fn select(
        &self,
        project: &KotlinProjectSemantics,
    ) -> Result<KotlinSemanticEngine, ClewError> {
        select_from_rows(project, QUALIFIED_COMPATIBILITY, true, None)
    }

    pub(crate) fn qualify(
        &self,
        project: &KotlinProjectSemantics,
        requested_engine: KotlinSemanticEngine,
    ) -> Result<KotlinSemanticEngine, ClewError> {
        select_from_rows(
            project,
            QUALIFIED_COMPATIBILITY,
            false,
            Some(requested_engine),
        )
    }

    pub fn next_untried_for_discovery(tried: u8) -> Option<KotlinSemanticEngine> {
        KotlinSemanticEngine::packaged_by_preference()
            .into_iter()
            .find(|engine| tried & engine.discovery_bit() == 0)
    }
}

fn select_from_rows(
    project: &KotlinProjectSemantics,
    rows: &[QualifiedCompatibility],
    default_only: bool,
    requested_engine: Option<KotlinSemanticEngine>,
) -> Result<KotlinSemanticEngine, ClewError> {
    let row = rows.iter().find(|row| {
        row.project_compiler_version == project.project_compiler_version
            && row
                .language_version
                .is_none_or(|version| project.language_version.as_deref() == Some(version))
            && row
                .api_version
                .is_none_or(|version| project.api_version.as_deref() == Some(version))
            && (!default_only || row.default_route)
            && requested_engine.is_none_or(|engine| row.engine == engine)
    });
    let Some(row) = row else {
        return Err(unsupported(
            "project Kotlin semantics have no checked-in qualified semantic engine",
        ));
    };
    if row.kind.is_cross_engine() {
        if !project.unstable_compiler_options.is_empty() {
            return Err(unsupported(
                "cross-engine Kotlin analysis has unqualified unstable compiler options",
            ));
        }
        let has_unknown_plugin = project
            .compiler_plugins
            .iter()
            .any(|plugin| plugin.kind == KotlinCompilerPluginKind::Unknown);
        if has_unknown_plugin {
            return Err(ClewError::new(
                ErrorCode::UnsupportedCompilerPluginAbi,
                "cross-engine Kotlin analysis has an unknown compiler plugin ABI",
            ));
        }
        let has_serialization = project
            .compiler_plugins
            .iter()
            .any(|plugin| plugin.kind == KotlinCompilerPluginKind::KotlinSerialization);
        if has_serialization && !row.allow_serialization_rebind {
            return Err(ClewError::new(
                ErrorCode::UnsupportedCompilerPluginAbi,
                "cross-engine Kotlin serialization plugin rebind is not qualified",
            ));
        }
    }
    Ok(row.engine)
}

fn unsupported(message: &str) -> ClewError {
    ClewError::new(ErrorCode::UnsupportedProjectConfiguration, message)
}

fn internal(error: impl std::fmt::Display) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(
        version: &str,
        language: &str,
        plugins: &[(&str, KotlinCompilerPluginKind)],
    ) -> KotlinProjectSemantics {
        KotlinProjectSemantics {
            schema: KOTLIN_PROJECT_SEMANTICS_SCHEMA.into(),
            project_compiler_version: version.into(),
            compiler_version_authority: "TEST".into(),
            language_version: Some(language.into()),
            api_version: Some(language.into()),
            jvm_target: Some("21".into()),
            compiler_plugins: plugins
                .iter()
                .map(|(name, kind)| KotlinCompilerPluginSemantics {
                    artifact_name: (*name).into(),
                    kind: kind.clone(),
                })
                .collect(),
            unstable_compiler_options: Vec::new(),
        }
    }

    #[test]
    fn current_exact_rows_remain_behavior_compatible() {
        let registry = KotlinEngineRegistry;
        assert_eq!(
            registry.select(&project("2.3.0", "2.3", &[])).unwrap(),
            KotlinSemanticEngine::Kotlin23
        );
        assert_eq!(
            registry.select(&project("2.4.10", "2.4", &[])).unwrap(),
            KotlinSemanticEngine::Kotlin24
        );
    }

    #[test]
    fn language_version_does_not_select_the_engine_line() {
        let registry = KotlinEngineRegistry;
        assert_eq!(
            registry.select(&project("2.4.10", "2.3", &[])).unwrap(),
            KotlinSemanticEngine::Kotlin24
        );
    }

    #[test]
    fn qualified_patch_line_keeps_cross_engine_safeguards() {
        let registry = KotlinEngineRegistry;
        let error = registry
            .select(&project(
                "2.4.0",
                "2.4",
                &[("custom-plugin.jar", KotlinCompilerPluginKind::Unknown)],
            ))
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::UnsupportedCompilerPluginAbi);

        let mut with_flag = project("2.4.0", "2.4", &[]);
        with_flag.unstable_compiler_options = vec!["-Xcontext-parameters".into()];
        let error = registry.select(&with_flag).unwrap_err();
        assert_eq!(error.code, ErrorCode::UnsupportedProjectConfiguration);
    }

    #[test]
    fn qualified_patch_line_is_a_production_route() {
        let registry = KotlinEngineRegistry;
        assert_eq!(
            registry.select(&project("2.4.0", "2.4", &[])).unwrap(),
            KotlinSemanticEngine::Kotlin24,
        );
        assert_eq!(
            registry
                .qualify(
                    &project("2.4.0", "2.4", &[]),
                    KotlinSemanticEngine::Kotlin24,
                )
                .unwrap(),
            KotlinSemanticEngine::Kotlin24,
        );
    }

    #[test]
    fn experimental_row_is_not_a_production_route() {
        let registry = KotlinEngineRegistry;
        let error = registry.select(&project("2.1.21", "2.1", &[])).unwrap_err();
        assert_eq!(error.code, ErrorCode::UnsupportedProjectConfiguration);
        assert_eq!(
            registry
                .qualify(
                    &project("2.1.21", "2.1", &[]),
                    KotlinSemanticEngine::Kotlin24,
                )
                .unwrap(),
            KotlinSemanticEngine::Kotlin24,
        );
        assert_eq!(
            registry
                .qualify(
                    &project("2.3.0", "2.3", &[]),
                    KotlinSemanticEngine::Kotlin24,
                )
                .unwrap(),
            KotlinSemanticEngine::Kotlin24,
        );
    }

    #[test]
    fn qualification_rejects_unlisted_versions_and_wrong_engine() {
        let registry = KotlinEngineRegistry;
        for (version, engine) in [
            ("1.9.24", KotlinSemanticEngine::Kotlin24),
            ("2.4.0", KotlinSemanticEngine::Kotlin23),
        ] {
            let error = registry
                .qualify(&project(version, "1.9", &[]), engine)
                .unwrap_err();
            assert_eq!(error.code, ErrorCode::UnsupportedProjectConfiguration);
        }
    }

    #[test]
    fn qualification_rejects_unknown_plugin_and_unstable_flags() {
        let registry = KotlinEngineRegistry;
        let with_plugin = project(
            "2.3.0",
            "2.3",
            &[("custom-plugin.jar", KotlinCompilerPluginKind::Unknown)],
        );
        let error = registry
            .qualify(&with_plugin, KotlinSemanticEngine::Kotlin24)
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::UnsupportedCompilerPluginAbi);

        let mut with_flag = project("2.3.0", "2.3", &[]);
        with_flag.unstable_compiler_options = vec!["-Xcontext-parameters".into()];
        let error = registry
            .qualify(&with_flag, KotlinSemanticEngine::Kotlin24)
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::UnsupportedProjectConfiguration);
    }

    #[test]
    fn serialization_is_an_explicit_qualification_capability() {
        let registry = KotlinEngineRegistry;
        assert_eq!(
            registry
                .qualify(
                    &project(
                        "2.1.21",
                        "2.1",
                        &[(
                            "kotlin-serialization-compiler-plugin-embeddable-2.1.21.jar",
                            KotlinCompilerPluginKind::KotlinSerialization,
                        )],
                    ),
                    KotlinSemanticEngine::Kotlin24,
                )
                .unwrap(),
            KotlinSemanticEngine::Kotlin24,
        );
    }

    #[test]
    fn project_semantics_uses_effective_plugins_not_kgp_implementation_jars() {
        let model = json!({
            "declaredCompilerVersion":"2.3.0",
            "languageVersion":"2.3",
            "apiVersion":"2.3",
            "jvmTarget":"21",
            "requestedCompilerPlugins":[
                "kotlin-scripting-compiler-embeddable-2.3.0.jar",
                "kotlin-stdlib-2.3.0.jar"
            ],
            "compilerPlugins":[],
        });
        let project = KotlinProjectSemantics::from_project_model(&model).unwrap();
        assert!(project.compiler_plugins.is_empty());
        assert_eq!(
            KotlinEngineRegistry
                .qualify(&project, KotlinSemanticEngine::Kotlin24)
                .unwrap(),
            KotlinSemanticEngine::Kotlin24,
        );
    }

    #[test]
    fn normalized_artifact_identity_preserves_serialization_policy() {
        let model = json!({
            "declaredCompilerVersion":"2.1.21",
            "languageVersion":"2.1",
            "apiVersion":"2.1",
            "jvmTarget":"21",
            "compilerPlugins":[format!(
                "artifact:kotlin-serialization-compiler-plugin-embeddable-2.4.10.jar:sha256:{}",
                "a".repeat(64),
            )],
        });
        let project = KotlinProjectSemantics::from_project_model(&model).unwrap();
        assert_eq!(
            project.compiler_plugins[0].kind,
            KotlinCompilerPluginKind::KotlinSerialization,
        );
        KotlinEngineRegistry
            .qualify(&project, KotlinSemanticEngine::Kotlin24)
            .unwrap();
    }

    #[test]
    fn cache_authority_contains_project_and_engine_identities() {
        let project = project("2.4.10", "2.3", &[]);
        assert_ne!(
            project
                .authority_digest(KotlinSemanticEngine::Kotlin23)
                .unwrap(),
            project
                .authority_digest(KotlinSemanticEngine::Kotlin24)
                .unwrap(),
        );
        let mut changed = project.clone();
        changed.language_version = Some("2.4".into());
        assert_ne!(
            project
                .authority_digest(KotlinSemanticEngine::Kotlin24)
                .unwrap(),
            changed
                .authority_digest(KotlinSemanticEngine::Kotlin24)
                .unwrap(),
        );
    }
}
