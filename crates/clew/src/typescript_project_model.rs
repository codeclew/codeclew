use crate::canonical;
use crate::error::{ClewError, ErrorCode};
use crate::typescript_adapter_v2::TypeScriptCompilerFact;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

pub const TYPESCRIPT_MODEL_SCHEMA: &str = "codeclew-typescript-project-model/1.0";
pub const JAVASCRIPT_MODEL_SCHEMA: &str = "codeclew-javascript-project-model/1.0";
const ANALYZER_OUTPUT_SCHEMA: &str = "codeclew-ecmascript-analyzer-output/1.1";
const ANALYZER_SOURCE: &str = include_str!("typescript_analyzer.cjs");
const MAX_ANALYZER_OUTPUT_BYTES: usize = 64 * 1024 * 1024;
const MAX_ANALYZER_ERROR_BYTES: usize = 4 * 1024 * 1024;
const MAX_SOURCE_FILES: usize = 32_768;
const MAX_EXTERNAL_FILES: usize = 65_536;
const MAX_EXTERNAL_FILE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_EXTERNAL_TOTAL_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TypeScriptExternalAuthority {
    pub logical_name: String,
    pub digest: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TypeScriptProjectModel {
    pub schema: String,
    pub model_digest: String,
    pub language: String,
    pub authority_mode: String,
    pub compilation: String,
    pub config_path: String,
    pub config_digest: String,
    pub compiler_version: String,
    pub compiler_module_digest: String,
    pub node_version: String,
    pub source_files: Vec<String>,
    pub external_files: Vec<TypeScriptExternalAuthority>,
    pub canonical_options: Value,
    pub project_references: Vec<String>,
    pub boundaries: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct TypeScriptOperationalModel {
    pub authority: TypeScriptProjectModel,
    pub facts: Vec<TypeScriptCompilerFact>,
    pub node_executable: PathBuf,
    pub typescript_module: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeScriptCompilationSelector {
    pub config_path: String,
}

impl TypeScriptCompilationSelector {
    pub fn parse(value: &str) -> Result<Self, ClewError> {
        let path = value
            .strip_prefix("tsconfig:")
            .filter(|path| !path.is_empty() && path.len() <= 512)
            .ok_or_else(|| invalid("TypeScript compilation selector is invalid"))?;
        if Path::new(path).is_absolute()
            || !path.ends_with(".json")
            || Path::new(path)
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(invalid("TypeScript compilation selector is invalid"));
        }
        Ok(Self {
            config_path: path.into(),
        })
    }

    pub fn canonical(&self) -> String {
        format!("tsconfig:{}", self.config_path)
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AnalyzerExternalFile {
    logical_name: String,
    physical_path: PathBuf,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AnalyzerOutput {
    schema: String,
    language: String,
    authority_mode: String,
    compiler_version: String,
    node_version: String,
    config_path: String,
    source_files: Vec<String>,
    external_files: Vec<AnalyzerExternalFile>,
    canonical_options: Value,
    project_references: Vec<String>,
    facts: Vec<TypeScriptCompilerFact>,
}

pub fn analyzer_digest() -> String {
    canonical::hash_bytes(ANALYZER_SOURCE.as_bytes())
}

pub fn extract_typescript_model(
    repository: &Path,
    compilation: &str,
) -> Result<TypeScriptOperationalModel, ClewError> {
    extract_ecmascript_model(repository, compilation, "typescript")
}

pub fn extract_javascript_model(
    repository: &Path,
    compilation: &str,
) -> Result<TypeScriptOperationalModel, ClewError> {
    extract_ecmascript_model(repository, compilation, "javascript")
}

fn extract_ecmascript_model(
    repository: &Path,
    compilation: &str,
    language: &str,
) -> Result<TypeScriptOperationalModel, ClewError> {
    let repository = repository.canonicalize().map_err(io_error)?;
    let selector = TypeScriptCompilationSelector::parse(compilation)?;
    let config = repository.join(&selector.config_path);
    let config_metadata = fs::symlink_metadata(&config)
        .map_err(|_| unsupported("selected TypeScript config is unavailable"))?;
    if !config_metadata.is_file() || config_metadata.file_type().is_symlink() {
        return Err(unsupported(
            "selected TypeScript config must be a regular project file",
        ));
    }
    let typescript_module = locate_typescript(&repository, &config)?;
    let typescript_root = typescript_module
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| unsupported("TypeScript package layout is invalid"))?;
    let package: Value = serde_json::from_slice(
        &fs::read(typescript_root.join("package.json"))
            .map_err(|_| unsupported("TypeScript package metadata is unavailable"))?,
    )
    .map_err(|_| unsupported("TypeScript package metadata is invalid"))?;
    let package_version = package
        .get("version")
        .and_then(Value::as_str)
        .filter(|version| version.starts_with("5."))
        .ok_or_else(|| unsupported("TypeScript v1 supports project TypeScript 5.x"))?;
    let module_bytes = bounded_file(&typescript_module, 32 * 1024 * 1024)?;
    let temporary = tempfile::Builder::new()
        .prefix("codeclew-typescript-analyzer-")
        .suffix(".cjs")
        .tempfile()
        .map_err(io_error)?;
    fs::write(temporary.path(), ANALYZER_SOURCE).map_err(io_error)?;
    let output = Command::new("node")
        .args([
            temporary
                .path()
                .to_str()
                .ok_or_else(|| internal("temporary analyzer path is not UTF-8"))?,
            repository
                .to_str()
                .ok_or_else(|| unsupported("TypeScript repository path is not UTF-8"))?,
            &selector.config_path,
            typescript_module
                .to_str()
                .ok_or_else(|| unsupported("TypeScript module path is not UTF-8"))?,
            language,
        ])
        .current_dir(&repository)
        .env_remove("NODE_OPTIONS")
        .env_remove("NODE_PATH")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|_| unsupported("Node.js could not start the TypeScript analyzer"))?;
    if !output.status.success()
        || output.stdout.is_empty()
        || output.stdout.len() > MAX_ANALYZER_OUTPUT_BYTES
        || output.stderr.len() > MAX_ANALYZER_ERROR_BYTES
    {
        return Err(ClewError::new(
            ErrorCode::IncompleteSemanticAnalysis,
            "TypeScript compiler analyzer did not produce bounded facts",
        ));
    }
    let mut observed: AnalyzerOutput = serde_json::from_slice(&output.stdout)
        .map_err(|_| corrupt("TypeScript analyzer output schema is invalid"))?;
    if observed.schema != ANALYZER_OUTPUT_SCHEMA
        || observed.language != language
        || !authority_mode_is_valid(language, &observed.authority_mode)
        || observed.config_path != selector.config_path
        || observed.compiler_version != package_version
        || observed.source_files.is_empty()
        || observed.source_files.len() > MAX_SOURCE_FILES
        || observed.external_files.len() > MAX_EXTERNAL_FILES
        || observed
            .source_files
            .iter()
            .chain(observed.project_references.iter())
            .any(|path| !safe_relative_path(path))
        || contains_private_path(&observed.canonical_options, &repository, typescript_root)
    {
        return Err(corrupt("TypeScript analyzer authority is invalid"));
    }
    observed.source_files.sort();
    observed.source_files.dedup();
    observed.project_references.sort();
    observed.project_references.dedup();
    let external_files = external_authorities(
        &repository,
        typescript_root,
        std::mem::take(&mut observed.external_files),
    )?;
    let mut boundaries = observed
        .facts
        .iter()
        .filter_map(TypeScriptCompilerFact::boundary_code)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    boundaries.sort();
    boundaries.dedup();
    let config_digest = canonical::hash_bytes(&bounded_file(&config, 4 * 1024 * 1024)?);
    let compiler_module_digest = canonical::hash_bytes(&module_bytes);
    let mut authority = TypeScriptProjectModel {
        schema: model_schema(language)?.into(),
        model_digest: String::new(),
        language: format!("language:{language}"),
        authority_mode: observed.authority_mode,
        compilation: selector.canonical(),
        config_path: selector.config_path,
        config_digest,
        compiler_version: observed.compiler_version,
        compiler_module_digest,
        node_version: observed.node_version,
        source_files: observed.source_files,
        external_files,
        canonical_options: observed.canonical_options,
        project_references: observed.project_references,
        boundaries,
    };
    authority.model_digest = model_digest(&authority)?;
    verify_model(&authority)?;
    let encoded_facts = serde_json::to_string(&observed.facts).map_err(internal)?;
    if encoded_facts.contains(repository.to_string_lossy().as_ref())
        || encoded_facts.contains(typescript_root.to_string_lossy().as_ref())
    {
        return Err(corrupt("TypeScript facts contain a private absolute path"));
    }
    Ok(TypeScriptOperationalModel {
        authority,
        facts: observed.facts,
        node_executable: PathBuf::from("node"),
        typescript_module,
    })
}

pub fn verify_model(model: &TypeScriptProjectModel) -> Result<(), ClewError> {
    let mut source_files = BTreeSet::new();
    let mut external_files = BTreeSet::new();
    let language = model
        .language
        .strip_prefix("language:")
        .ok_or_else(|| corrupt("ECMAScript project model language is invalid"))?;
    if model.schema != model_schema(language)?
        || !authority_mode_is_valid(language, &model.authority_mode)
        || model.model_digest != model_digest(model)?
        || TypeScriptCompilationSelector::parse(&model.compilation)?.config_path
            != model.config_path
        || !digest(&model.config_digest)
        || !digest(&model.compiler_module_digest)
        || !model.compiler_version.starts_with("5.")
        || !model.node_version.starts_with('v')
        || model.source_files.is_empty()
        || model
            .source_files
            .iter()
            .any(|path| !safe_relative_path(path) || !source_files.insert(path))
        || model.external_files.iter().any(|file| {
            !safe_relative_path(&file.logical_name)
                || !digest(&file.digest)
                || file.size > MAX_EXTERNAL_FILE_BYTES
                || !external_files.insert(file.logical_name.as_str())
        })
        || model
            .project_references
            .iter()
            .any(|path| !safe_relative_path(path))
        || model.boundaries.windows(2).any(|pair| pair[0] >= pair[1])
    {
        return Err(corrupt("TypeScript project model authority is invalid"));
    }
    Ok(())
}

fn model_schema(language: &str) -> Result<&'static str, ClewError> {
    match language {
        "typescript" => Ok(TYPESCRIPT_MODEL_SCHEMA),
        "javascript" => Ok(JAVASCRIPT_MODEL_SCHEMA),
        _ => Err(corrupt("ECMAScript project model language is invalid")),
    }
}

fn authority_mode_is_valid(language: &str, mode: &str) -> bool {
    match language {
        "typescript" => mode == "TYPESCRIPT_CHECKED",
        "javascript" => matches!(
            mode,
            "JAVASCRIPT_CHECKED" | "JAVASCRIPT_DECLARATION_TYPED" | "JAVASCRIPT_SYNTAX_CONDITIONAL"
        ),
        _ => false,
    }
}

fn locate_typescript(repository: &Path, config: &Path) -> Result<PathBuf, ClewError> {
    let mut directory = config.parent();
    while let Some(candidate_root) = directory {
        if !candidate_root.starts_with(repository) {
            break;
        }
        let candidate = candidate_root.join("node_modules/typescript/lib/typescript.js");
        if candidate.is_file() {
            let canonical = candidate.canonicalize().map_err(io_error)?;
            if !canonical.starts_with(repository) {
                return Err(unsupported(
                    "project TypeScript module resolves outside the repository",
                ));
            }
            return Ok(canonical);
        }
        if candidate_root == repository {
            break;
        }
        directory = candidate_root.parent();
    }
    Err(unsupported(
        "TypeScript profile requires project-local node_modules/typescript",
    ))
}

fn external_authorities(
    repository: &Path,
    typescript_root: &Path,
    files: Vec<AnalyzerExternalFile>,
) -> Result<Vec<TypeScriptExternalAuthority>, ClewError> {
    let mut total = 0u64;
    let mut observed = BTreeSet::new();
    let mut authorities = Vec::with_capacity(files.len());
    for file in files {
        if !safe_relative_path(&file.logical_name) || !observed.insert(file.logical_name.clone()) {
            continue;
        }
        let path = file
            .physical_path
            .canonicalize()
            .map_err(|_| unsupported("TypeScript declaration dependency became unavailable"))?;
        if !path.starts_with(repository) && !path.starts_with(typescript_root) {
            return Err(unsupported(
                "TypeScript declaration dependency resolves outside project authority",
            ));
        }
        let bytes = bounded_file(&path, MAX_EXTERNAL_FILE_BYTES as usize)?;
        total = total.saturating_add(bytes.len() as u64);
        if total > MAX_EXTERNAL_TOTAL_BYTES {
            return Err(resource(
                "TypeScript declaration dependency set exceeds 512 MiB",
            ));
        }
        authorities.push(TypeScriptExternalAuthority {
            logical_name: file.logical_name,
            digest: canonical::hash_bytes(&bytes),
            size: bytes.len() as u64,
        });
    }
    authorities.sort_by(|left, right| left.logical_name.cmp(&right.logical_name));
    Ok(authorities)
}

fn contains_private_path(value: &Value, repository: &Path, typescript_root: &Path) -> bool {
    serde_json::to_string(value).is_ok_and(|encoded| {
        encoded.contains(repository.to_string_lossy().as_ref())
            || encoded.contains(typescript_root.to_string_lossy().as_ref())
    })
}

fn bounded_file(path: &Path, limit: usize) -> Result<Vec<u8>, ClewError> {
    let metadata = fs::metadata(path).map_err(io_error)?;
    if !metadata.is_file() || metadata.len() > limit as u64 {
        return Err(resource(
            "TypeScript authority file exceeds its byte budget",
        ));
    }
    let bytes = fs::read(path).map_err(io_error)?;
    if bytes.len() > limit {
        return Err(resource(
            "TypeScript authority file exceeds its byte budget",
        ));
    }
    Ok(bytes)
}

fn model_digest(model: &TypeScriptProjectModel) -> Result<String, ClewError> {
    let mut unsigned = model.clone();
    unsigned.model_digest.clear();
    canonical::hash(&unsigned).map_err(internal)
}

fn safe_relative_path(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 4096
        && !Path::new(value).is_absolute()
        && Path::new(value)
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    })
}

fn invalid(message: &str) -> ClewError {
    ClewError::new(ErrorCode::InvalidInput, message)
}

fn unsupported(message: &str) -> ClewError {
    ClewError::new(ErrorCode::UnsupportedProjectConfiguration, message)
}

fn corrupt(message: &str) -> ClewError {
    ClewError::new(ErrorCode::StateCorrupt, message)
}

fn resource(message: &str) -> ClewError {
    ClewError::new(ErrorCode::ResourceLimit, message)
}

fn internal(error: impl std::fmt::Display) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}

fn io_error(error: std::io::Error) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compilation_selector_is_exact_and_repository_relative() {
        assert_eq!(
            TypeScriptCompilationSelector::parse("tsconfig:packages/api/tsconfig.json")
                .unwrap()
                .canonical(),
            "tsconfig:packages/api/tsconfig.json"
        );
        for invalid in [
            "tsconfig:/private/tsconfig.json",
            "tsconfig:../tsconfig.json",
            "tsconfig:tsconfig.ts",
            "tsconfig:",
            ":/main",
        ] {
            assert!(TypeScriptCompilationSelector::parse(invalid).is_err());
        }
    }

    #[test]
    fn analyzer_and_model_authority_are_content_bound() {
        assert!(digest(&analyzer_digest()));
        assert_eq!(
            analyzer_digest(),
            canonical::hash_bytes(include_str!("typescript_analyzer.cjs").as_bytes())
        );
    }

    #[test]
    fn javascript_authority_modes_are_explicit_and_closed() {
        for mode in [
            "JAVASCRIPT_CHECKED",
            "JAVASCRIPT_DECLARATION_TYPED",
            "JAVASCRIPT_SYNTAX_CONDITIONAL",
        ] {
            assert!(authority_mode_is_valid("javascript", mode));
        }
        assert!(!authority_mode_is_valid("javascript", "TYPESCRIPT_CHECKED"));
        assert!(!authority_mode_is_valid("javascript", "UNKNOWN"));
        assert!(authority_mode_is_valid("typescript", "TYPESCRIPT_CHECKED"));
    }
}
