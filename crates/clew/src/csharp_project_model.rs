//! C# compiler authority from the optional Roslyn worker.
//!
//! The worker runs the project's design-time `Compile` target in-process with the
//! repository-selected .NET SDK, builds Roslyn compilations from the exact csc
//! command lines, and writes NDJSON facts. It never restores packages and
//! redirects MSBuild output into a private scratch directory; this module also
//! checks that no `obj/` or `bin/` entry of the target changed.
use crate::canonical;
use crate::csharp_adapter_v2::CSharpCompilerFact;
use crate::error::{ClewError, ErrorCode};
use crate::runtime::{CSHARP_WORKER, RuntimeAuthority};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

pub const CSHARP_MODEL_SCHEMA: &str = "codeclew-csharp-project-model/1.0";
const REQUEST_SCHEMA: &str = "codeclew-csharp-analyzer-request/1.0";
const OUTPUT_SCHEMA: &str = "codeclew-csharp-analyzer-output/1.0";
const WORKER_ASSEMBLY: &str = "Codeclew.CSharp.Analyzer.dll";
const AUTHORITY_MODE: &str = "MSBUILD_DESIGN_TIME_COMMAND_LINE";
const MAX_FACTS: usize = 1_048_576;
const MAX_LINE_BYTES: usize = 512 * 1024;
const MAX_HEADER_BYTES: usize = 64 * 1024 * 1024;
const MAX_OUTPUT_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_STATUS_BYTES: usize = 64 * 1024;
const MAX_STDERR_BYTES: usize = 4 * 1024 * 1024;
const MAX_GUARDED_ENTRIES: usize = 2_000_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CSharpCompilationSelector {
    pub kind: CSharpSelectorKind,
    pub path: String,
    pub target_framework: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CSharpSelectorKind {
    Project,
    Solution,
}

impl CSharpCompilationSelector {
    /// `csproj:<path>[@<tfm>]` or `sln:<path>` (`.sln`/`.slnx`), repository-relative.
    pub fn parse(value: &str) -> Result<Self, ClewError> {
        let (kind, rest) = if let Some(rest) = value.strip_prefix("csproj:") {
            (CSharpSelectorKind::Project, rest)
        } else if let Some(rest) = value.strip_prefix("sln:") {
            (CSharpSelectorKind::Solution, rest)
        } else {
            return Err(invalid(
                "C# compilation selector must start with csproj: or sln:",
            ));
        };
        let (path, target_framework) = match (kind, rest.rsplit_once('@')) {
            (CSharpSelectorKind::Project, Some((path, framework))) if !path.is_empty() => {
                if framework.is_empty()
                    || framework.len() > 64
                    || !framework
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
                {
                    return Err(invalid("C# target framework selector is invalid"));
                }
                (path, Some(framework.to_owned()))
            }
            _ => (rest, None),
        };
        let extension_valid = match kind {
            CSharpSelectorKind::Project => path.ends_with(".csproj"),
            CSharpSelectorKind::Solution => path.ends_with(".sln") || path.ends_with(".slnx"),
        };
        // `Path::components` drops `.` segments, so spelling is checked per segment.
        if !extension_valid
            || path.len() > 512
            || path.contains('\\')
            || path
                .split('/')
                .any(|segment| matches!(segment, "" | "." | ".."))
            || !safe_relative_path(path)
        {
            return Err(invalid("C# compilation selector path is invalid"));
        }
        Ok(Self {
            kind,
            path: path.into(),
            target_framework,
        })
    }

    pub fn canonical(&self) -> String {
        match (self.kind, &self.target_framework) {
            (CSharpSelectorKind::Project, Some(framework)) => {
                format!("csproj:{}@{framework}", self.path)
            }
            (CSharpSelectorKind::Project, None) => format!("csproj:{}", self.path),
            (CSharpSelectorKind::Solution, _) => format!("sln:{}", self.path),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CSharpReferenceAuthority {
    pub logical_name: String,
    pub digest: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CSharpProjectAuthority {
    pub path: String,
    pub assembly_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_framework: Option<String>,
    pub lang_version: String,
    pub nullable: String,
    pub define_constants: Vec<String>,
    pub source_files: u64,
    pub generated_documents: u64,
    pub metadata_references: Vec<CSharpReferenceAuthority>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CSharpProjectModel {
    pub schema: String,
    pub model_digest: String,
    pub language: String,
    pub authority_mode: String,
    pub compilation: String,
    pub selector_path: String,
    pub selector_digest: String,
    pub sdk_version: String,
    pub msbuild_version: String,
    pub analyzer_roslyn_version: String,
    pub worker_tree_hash: String,
    pub projects: Vec<CSharpProjectAuthority>,
    pub unrestored_projects: Vec<String>,
    pub source_files: Vec<String>,
    pub boundaries: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct CSharpOperationalModel {
    pub authority: CSharpProjectModel,
    pub facts: Vec<CSharpCompilerFact>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AnalyzerHeader {
    schema: String,
    protocol: String,
    language: String,
    authority_mode: String,
    compilation: String,
    analyzer_roslyn_version: String,
    msbuild_version: Option<String>,
    sdk_version: Option<String>,
    runtime_version: String,
    projects: Vec<CSharpProjectAuthority>,
    unrestored_projects: Vec<String>,
    source_files: Vec<String>,
    boundaries: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AnalyzerTrailer {
    kind: String,
    fact_count: usize,
    facts_digest: String,
}

/// The optional runtime component that hosts the Roslyn analyzer.
pub fn worker_assembly(runtime: &RuntimeAuthority) -> Result<(PathBuf, String), ClewError> {
    let worker = runtime.workers.get(CSHARP_WORKER).ok_or_else(|| {
        ClewError::new(
            ErrorCode::WorkerPreparationRequired,
            "the C# Roslyn worker is not installed; install a .NET 10 SDK and rebuild the runtime",
        )
    })?;
    let root = runtime.verify_worker(CSHARP_WORKER)?;
    let assembly = root.join(WORKER_ASSEMBLY);
    if !assembly.is_file() {
        return Err(corrupt(
            "C# worker distribution lacks its analyzer assembly",
        ));
    }
    Ok((assembly, worker.tree_hash.clone()))
}

pub fn extract_csharp_model(
    runtime: &RuntimeAuthority,
    repository: &Path,
    compilation: &str,
) -> Result<CSharpOperationalModel, ClewError> {
    let (assembly, worker_tree_hash) = worker_assembly(runtime)?;
    extract_with_worker(&assembly, &worker_tree_hash, repository, compilation)
}

pub(crate) fn extract_with_worker(
    assembly: &Path,
    worker_tree_hash: &str,
    repository: &Path,
    compilation: &str,
) -> Result<CSharpOperationalModel, ClewError> {
    let repository = repository.canonicalize().map_err(io_error)?;
    let selector = CSharpCompilationSelector::parse(compilation)?;
    let selected = repository.join(&selector.path);
    let selected_metadata = fs::symlink_metadata(&selected)
        .map_err(|_| unsupported("selected C# project or solution is unavailable"))?;
    if !selected_metadata.is_file() || selected_metadata.file_type().is_symlink() {
        return Err(unsupported(
            "selected C# project or solution must be a regular file",
        ));
    }
    let selector_digest = canonical::hash_bytes(&bounded_file(&selected, 16 * 1024 * 1024)?);
    let scratch = tempfile::Builder::new()
        .prefix("codeclew-csharp-")
        .tempdir()
        .map_err(io_error)?;
    let output = scratch.path().join("facts.ndjson");
    let work = scratch.path().join("work");
    let home = scratch.path().join("home");
    fs::create_dir(&work).map_err(io_error)?;
    fs::create_dir(&home).map_err(io_error)?;
    let request = json!({
        "schema":REQUEST_SCHEMA,
        "repository":path_text(&repository)?,
        "compilation":selector.canonical(),
        "output":path_text(&output)?,
        "scratch":path_text(&work)?,
        "maxFacts":MAX_FACTS,
    });
    let before = build_output_state(&repository)?;
    let status = run_worker(assembly, &repository, &home, &request)?;
    if build_output_state(&repository)? != before {
        return Err(ClewError::new(
            ErrorCode::InputMutated,
            "C# analysis changed a project obj/ or bin/ directory of the target",
        ));
    }
    match status["status"].as_str() {
        Some("OK") => {}
        Some("UNSUPPORTED") => {
            return Err(unsupported(&format!(
                "C# analysis is unsupported for this project ({}): {}",
                status["code"].as_str().unwrap_or("CSHARP_UNSUPPORTED"),
                status["message"].as_str().unwrap_or(""),
            )));
        }
        _ => {
            return Err(ClewError::new(
                ErrorCode::IncompleteSemanticAnalysis,
                format!(
                    "C# Roslyn analyzer failed ({})",
                    status["code"].as_str().unwrap_or("CSHARP_ANALYZER_FAILED")
                ),
            ));
        }
    }
    let (header, facts) = read_output(&output, &repository, &work)?;
    if header.schema != OUTPUT_SCHEMA
        || header.protocol != crate::runtime::CSHARP_WORKER_PROTOCOL
        || header.language != "csharp"
        || header.authority_mode != AUTHORITY_MODE
        || header.compilation != selector.canonical()
        || header.source_files.is_empty()
        || header
            .source_files
            .iter()
            .any(|path| !safe_relative_path(path))
        || header.projects.is_empty()
        || header.runtime_version.is_empty()
    {
        return Err(corrupt("C# analyzer header authority is invalid"));
    }
    let mut boundaries = header.boundaries.clone();
    boundaries.extend(
        facts
            .iter()
            .filter_map(CSharpCompilerFact::boundary_code)
            .map(str::to_owned),
    );
    boundaries.sort();
    boundaries.dedup();
    let mut projects = header.projects;
    projects.sort_by(|left, right| left.path.cmp(&right.path));
    let mut unrestored = header.unrestored_projects;
    unrestored.sort();
    unrestored.dedup();
    let mut source_files = header.source_files;
    source_files.sort();
    source_files.dedup();
    let mut authority = CSharpProjectModel {
        schema: CSHARP_MODEL_SCHEMA.into(),
        model_digest: String::new(),
        language: "language:csharp".into(),
        authority_mode: header.authority_mode,
        compilation: selector.canonical(),
        selector_path: selector.path,
        selector_digest,
        sdk_version: header.sdk_version.unwrap_or_default(),
        msbuild_version: header.msbuild_version.unwrap_or_default(),
        analyzer_roslyn_version: header.analyzer_roslyn_version,
        worker_tree_hash: worker_tree_hash.into(),
        projects,
        unrestored_projects: unrestored,
        source_files,
        boundaries,
    };
    authority.model_digest = model_digest(&authority)?;
    verify_model(&authority)?;
    Ok(CSharpOperationalModel { authority, facts })
}

pub fn verify_model(model: &CSharpProjectModel) -> Result<(), ClewError> {
    let mut sources = BTreeSet::new();
    if model.schema != CSHARP_MODEL_SCHEMA
        || model.language != "language:csharp"
        || model.authority_mode != AUTHORITY_MODE
        || model.model_digest != model_digest(model)?
        || CSharpCompilationSelector::parse(&model.compilation)?.path != model.selector_path
        || !digest(&model.selector_digest)
        || !digest(&model.worker_tree_hash)
        || model.analyzer_roslyn_version.is_empty()
        || model.projects.is_empty()
        || model.source_files.is_empty()
        || model
            .source_files
            .iter()
            .any(|path| !safe_relative_path(path) || !sources.insert(path))
        || model.projects.iter().any(|project| {
            project.path.is_empty()
                || project.metadata_references.iter().any(|reference| {
                    !digest(&reference.digest) || reference.logical_name.starts_with('/')
                })
        })
        || model.boundaries.windows(2).any(|pair| pair[0] >= pair[1])
    {
        return Err(corrupt("C# project model authority is invalid"));
    }
    Ok(())
}

fn run_worker(
    assembly: &Path,
    repository: &Path,
    home: &Path,
    request: &Value,
) -> Result<Value, ClewError> {
    let mut child = Command::new("dotnet")
        .arg(assembly)
        .current_dir(repository)
        .env("DOTNET_CLI_HOME", home)
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .env("DOTNET_NOLOGO", "1")
        .env("DOTNET_SKIP_FIRST_TIME_EXPERIENCE", "1")
        .env("DOTNET_CLI_USE_MSBUILD_SERVER", "0")
        .env("MSBUILDDISABLENODEREUSE", "1")
        .env_remove("MSBuildExtensionsPath")
        .env_remove("MSBUILD_EXE_PATH")
        .env_remove("MSBuildSDKsPath")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| {
            ClewError::new(
                ErrorCode::WorkerPreparationRequired,
                "dotnet could not start the C# Roslyn worker; install a .NET 10 SDK",
            )
        })?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| internal("C# worker stderr is unavailable"))?;
    let stderr_reader = std::thread::spawn(move || {
        let mut retained = Vec::new();
        let mut buffer = [0u8; 8192];
        while let Ok(read) = stderr.read(&mut buffer) {
            if read == 0 {
                break;
            }
            let room = MAX_STDERR_BYTES.saturating_sub(retained.len());
            retained.extend_from_slice(&buffer[..read.min(room)]);
        }
        retained
    });
    {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| internal("C# worker stdin is unavailable"))?;
        stdin
            .write_all(&canonical::bytes(request).map_err(internal)?)
            .map_err(io_error)?;
    }
    let mut stdout = Vec::new();
    child
        .stdout
        .take()
        .ok_or_else(|| internal("C# worker stdout is unavailable"))?
        .take(MAX_STATUS_BYTES as u64 + 1)
        .read_to_end(&mut stdout)
        .map_err(io_error)?;
    let exit = child.wait().map_err(io_error)?;
    let stderr = stderr_reader.join().unwrap_or_default();
    if std::env::var_os("CODECLEW_CSHARP_WORKER_STDERR").is_some() {
        let _ = std::io::stderr().write_all(&stderr);
    }
    if stdout.len() > MAX_STATUS_BYTES {
        return Err(corrupt("C# worker status exceeds its byte budget"));
    }
    let status: Value = stdout
        .split(|byte| *byte == b'\n')
        .find(|line| !line.is_empty())
        .and_then(|line| serde_json::from_slice(line).ok())
        .ok_or_else(|| {
            ClewError::new(
                ErrorCode::IncompleteSemanticAnalysis,
                format!(
                    "C# Roslyn worker exited without status ({})",
                    exit.code().map_or("signal".into(), |code| code.to_string())
                ),
            )
        })?;
    if status["protocol"] != crate::runtime::CSHARP_WORKER_PROTOCOL {
        return Err(corrupt("C# worker status protocol is invalid"));
    }
    let expected_exit = match status["status"].as_str() {
        Some("OK") => 0,
        Some("UNSUPPORTED") => 2,
        _ => 3,
    };
    if exit.code() != Some(expected_exit) {
        return Err(corrupt("C# worker exit status differs from its report"));
    }
    Ok(status)
}

fn read_output(
    path: &Path,
    repository: &Path,
    scratch: &Path,
) -> Result<(AnalyzerHeader, Vec<CSharpCompilerFact>), ClewError> {
    let metadata = fs::symlink_metadata(path).map_err(io_error)?;
    if !metadata.is_file() || metadata.len() > MAX_OUTPUT_BYTES {
        return Err(resource("C# analyzer output exceeds its byte budget"));
    }
    let private = [
        path_text(repository)?.into_bytes(),
        path_text(scratch)?.into_bytes(),
    ];
    let mut reader = BufReader::new(fs::File::open(path).map_err(io_error)?);
    let mut line = Vec::new();
    let mut read_line = |line: &mut Vec<u8>, limit: usize| -> Result<bool, ClewError> {
        line.clear();
        let read = (&mut reader)
            .take(limit as u64 + 1)
            .read_until(b'\n', line)
            .map_err(io_error)?;
        if read == 0 {
            return Ok(false);
        }
        if line.last() != Some(&b'\n') {
            return Err(corrupt(
                "C# analyzer output line is unterminated or too long",
            ));
        }
        line.pop();
        Ok(true)
    };
    if !read_line(&mut line, MAX_HEADER_BYTES)? {
        return Err(corrupt("C# analyzer output is empty"));
    }
    if private.iter().any(|value| contains(&line, value)) {
        return Err(corrupt(
            "C# analyzer header contains a private absolute path",
        ));
    }
    let header: AnalyzerHeader = serde_json::from_slice(&line)
        .map_err(|_| corrupt("C# analyzer header schema is invalid"))?;
    let mut hasher = Sha256::new();
    let mut facts = Vec::new();
    loop {
        if !read_line(&mut line, MAX_LINE_BYTES)? {
            return Err(corrupt("C# analyzer output has no trailer"));
        }
        if line.starts_with(br#"{"kind":"TRAILER""#) {
            break;
        }
        if facts.len() >= MAX_FACTS {
            return Err(resource("C# analysis exceeds its fact budget"));
        }
        if private.iter().any(|value| contains(&line, value)) {
            return Err(corrupt("C# facts contain a private absolute path"));
        }
        hasher.update(&line);
        hasher.update(b"\n");
        facts.push(
            serde_json::from_slice::<CSharpCompilerFact>(&line).map_err(|error| {
                corrupt(&format!("C# fact violates its closed schema: {error}"))
            })?,
        );
    }
    let trailer: AnalyzerTrailer =
        serde_json::from_slice(&line).map_err(|_| corrupt("C# analyzer trailer is invalid"))?;
    if trailer.kind != "TRAILER"
        || trailer.fact_count != facts.len()
        || trailer.facts_digest != format!("sha256:{}", hex::encode(hasher.finalize()))
        || read_line(&mut line, 1)?
    {
        return Err(corrupt("C# analyzer trailer does not bind its facts"));
    }
    Ok((header, facts))
}

/// Path, size and modification time of every entry below a project `obj/` or
/// `bin/` directory. Both are ignored by Git, so the snapshot capture cannot
/// observe writes there.
pub(crate) fn build_output_state(repository: &Path) -> Result<BTreeMap<String, String>, ClewError> {
    use std::os::unix::fs::MetadataExt;
    let mut state = BTreeMap::new();
    let mut pending = vec![(repository.to_path_buf(), false)];
    while let Some((directory, inside)) = pending.pop() {
        for entry in fs::read_dir(&directory).map_err(io_error)? {
            let entry = entry.map_err(io_error)?;
            let name = entry.file_name();
            if name == ".git" {
                continue;
            }
            let metadata = entry.path().symlink_metadata().map_err(io_error)?;
            let nested = inside || name == "obj" || name == "bin";
            if nested {
                let relative = entry
                    .path()
                    .strip_prefix(repository)
                    .map_err(internal)?
                    .to_string_lossy()
                    .into_owned();
                state.insert(
                    relative,
                    format!(
                        "{}:{}:{}",
                        metadata.mode(),
                        metadata.len(),
                        metadata.mtime_nsec() + metadata.mtime() * 1_000_000_000
                    ),
                );
                if state.len() > MAX_GUARDED_ENTRIES {
                    return Err(resource(
                        "C# target build output exceeds the write-guard budget",
                    ));
                }
            }
            if metadata.is_dir() && !metadata.file_type().is_symlink() {
                pending.push((entry.path(), nested));
            }
        }
    }
    Ok(state)
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

fn path_text(path: &Path) -> Result<String, ClewError> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| unsupported("C# analysis requires UTF-8 paths"))
}

fn bounded_file(path: &Path, limit: usize) -> Result<Vec<u8>, ClewError> {
    let metadata = fs::metadata(path).map_err(io_error)?;
    if !metadata.is_file() || metadata.len() > limit as u64 {
        return Err(resource("C# authority file exceeds its byte budget"));
    }
    fs::read(path).map_err(io_error)
}

fn model_digest(model: &CSharpProjectModel) -> Result<String, ClewError> {
    let mut unsigned = model.clone();
    unsigned.model_digest.clear();
    canonical::hash(&unsigned).map_err(internal)
}

pub(crate) fn safe_relative_path(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 4096
        && !Path::new(value).is_absolute()
        && Path::new(value)
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

pub(crate) fn digest(value: &str) -> bool {
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
        for (input, canonical) in [
            ("csproj:src/Api/Api.csproj", "csproj:src/Api/Api.csproj"),
            (
                "csproj:src/Api/Api.csproj@net8.0",
                "csproj:src/Api/Api.csproj@net8.0",
            ),
            ("sln:src/All.sln", "sln:src/All.sln"),
            ("sln:All.slnx", "sln:All.slnx"),
        ] {
            assert_eq!(
                CSharpCompilationSelector::parse(input).unwrap().canonical(),
                canonical
            );
        }
        for rejected in [
            "csproj:/abs/Api.csproj",
            "csproj:../Api.csproj",
            "csproj:src/./Api.csproj",
            "csproj:src\\Api.csproj",
            "csproj:Api.fsproj",
            "csproj:Api.csproj@",
            "csproj:Api.csproj@net 8",
            "sln:All.sln@net8.0",
            "tsconfig:tsconfig.json",
            ":app/main",
        ] {
            assert!(
                CSharpCompilationSelector::parse(rejected).is_err(),
                "{rejected}"
            );
        }
    }

    #[test]
    fn build_output_guard_observes_ignored_obj_and_bin_entries() {
        let repository = tempfile::tempdir().unwrap();
        let project = repository.path().join("src/Api");
        fs::create_dir_all(project.join("obj")).unwrap();
        fs::write(project.join("Api.cs"), "class A {}").unwrap();
        fs::write(project.join("obj/project.assets.json"), "{}").unwrap();
        let before = build_output_state(repository.path()).unwrap();
        assert!(before.contains_key("src/Api/obj/project.assets.json"));
        assert!(!before.contains_key("src/Api/Api.cs"));
        fs::write(project.join("Api.cs"), "class B {}").unwrap();
        assert_eq!(build_output_state(repository.path()).unwrap(), before);
        fs::create_dir_all(project.join("bin/Debug")).unwrap();
        assert_ne!(build_output_state(repository.path()).unwrap(), before);
    }

    /// Published worker: `CODECLEW_TEST_CSHARP_WORKER` or `workers/dotnet/publish`
    /// (`dotnet publish workers/dotnet/src -c Release -o workers/dotnet/publish`).
    #[test]
    #[ignore = "requires a .NET 10 SDK and the published C# worker"]
    fn fixture_solution_yields_roslyn_facts_routes_and_restore_boundaries() {
        let assembly = std::env::var_os("CODECLEW_TEST_CSHARP_WORKER")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                crate::worker::workspace_root()
                    .join("workers/dotnet/publish/Codeclew.CSharp.Analyzer.dll")
            });
        let fixture = crate::worker::workspace_root().join("fixtures/csharp-aspnetcore");
        let repository = tempfile::tempdir().unwrap();
        for entry in walkdir::WalkDir::new(&fixture) {
            let entry = entry.unwrap();
            let target = repository
                .path()
                .join(entry.path().strip_prefix(&fixture).unwrap());
            if entry.file_type().is_dir() {
                fs::create_dir_all(&target).unwrap();
            } else {
                fs::copy(entry.path(), &target).unwrap();
            }
        }
        // Restore belongs to test setup; the analyzer never restores. The test
        // project stays unrestorable because NuGet.config declares no sources.
        let restored = Command::new("dotnet")
            .args(["restore", "src/Orders.Api/Orders.Api.csproj"])
            .current_dir(repository.path())
            .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
            .env("DOTNET_NOLOGO", "1")
            .output()
            .unwrap();
        assert!(restored.status.success(), "{restored:?}");
        let model = extract_with_worker(
            &assembly,
            &format!("sha256:{}", "a".repeat(64)),
            repository.path(),
            "sln:Orders.slnx",
        )
        .unwrap();
        assert_eq!(
            model.authority.unrestored_projects,
            vec!["tests/Orders.Tests/Orders.Tests.csproj"]
        );
        assert!(
            model
                .authority
                .boundaries
                .contains(&"CSHARP_PROJECT_UNRESTORED".to_owned())
        );
        assert!(
            model
                .authority
                .boundaries
                .contains(&"GENERATED_SOURCE_BODIES_NOT_INDEXED".to_owned())
        );
        let values = model
            .facts
            .iter()
            .map(|fact| serde_json::to_value(fact).unwrap())
            .collect::<Vec<_>>();
        assert!(values.iter().any(|fact| fact["kind"] == "RELATION"
            && fact["targetIdentity"]
                == "method:class:Orders.Core.IOrderRepository#Save(LOrders/Core/Order;)V"));
        let mut routes = BTreeSet::new();
        for fact in &values {
            if let Some(metadata) = crate::aspnetcore_entrypoints::metadata_for_fact(fact).unwrap()
            {
                for entry in metadata.entries {
                    let methods = entry.trigger["methods"].as_array().unwrap().clone();
                    for path in entry.trigger["paths"].as_array().unwrap() {
                        for method in &methods {
                            routes.insert(format!(
                                "{} {}",
                                method.as_str().unwrap(),
                                path.as_str().unwrap()
                            ));
                        }
                    }
                }
            }
        }
        assert_eq!(
            routes,
            BTreeSet::from(
                [
                    "DELETE /internal/audit/{id}",
                    "GET /admin/Reports/Daily",
                    "GET /api/Orders/{id:int}",
                    "GET /api/v{version:apiVersion}/quotes",
                    "GET /health",
                    "POST /api/Orders",
                    "PUT /api/Orders/{id}",
                ]
                .map(str::to_owned)
            )
        );
    }
}
