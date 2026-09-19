//! Closed, read-only inputs for the reusable Java compiler analyzer.
//!
//! This module deliberately owns only request-scoped materialization. Durable
//! publication and generation-history binding remain generation-service work.

use crate::canonical;
use crate::error::{ClewError, ErrorCode};
use crate::java_analysis_scratch::{self, JavaAnalysisScratch};
use crate::java_project_model::{JavaBuildSystem, JavaClasspathAuthority, JavaOperationalModel};
use crate::repository_snapshot::seal_tree;
use crate::state::StateAuthority;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub const JAVA_ANALYSIS_INPUTS_SCHEMA: &str = "codeclew-java-analysis-inputs/1.0";
pub const JAVA_ANALYSIS_POLICY: &str = "JAVA_CLOSED_NO_AP_JDK21_MACOS_V1";
const JAVA_ANALYSIS_POLICY_JDK17: &str = "JAVA_CLOSED_NO_AP_JDK17_MACOS_V1";

const MAX_JDK_FILES: usize = 262_144;
const MAX_JDK_BYTES: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JavaPreparedRefusal {
    WritableTransform,
    ProcessorInputsPresent,
    UnsupportedAnalyzerOptions,
    UnqualifiedExecutionImage,
    UnsealedClasspath,
    ImplicitOrExternalLookup,
    LegacyInputAuthority,
}

impl JavaPreparedRefusal {
    pub const fn code(self) -> &'static str {
        match self {
            Self::WritableTransform => "WRITABLE_TRANSFORM",
            Self::ProcessorInputsPresent => "PROCESSOR_INPUTS_PRESENT",
            Self::UnsupportedAnalyzerOptions => "UNSUPPORTED_ANALYZER_OPTIONS_OR_MODULE_MODE",
            Self::UnqualifiedExecutionImage => "UNQUALIFIED_EXECUTION_IMAGE",
            Self::UnsealedClasspath => "UNSEALED_CLASSPATH",
            Self::ImplicitOrExternalLookup => "IMPLICIT_OR_EXTERNAL_LOOKUP",
            Self::LegacyInputAuthority => "LEGACY_INPUT_AUTHORITY",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JavaPreparedSourceAuthority {
    pub path: String,
    pub digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JavaPreparedClasspathAuthority {
    pub authority: JavaClasspathAuthority,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JavaPreparedJdkFile {
    pub path: String,
    pub kind: String,
    pub mode: u32,
    pub size: u64,
    pub digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_target: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JavaPreparedJdkAuthority {
    pub image_digest: String,
    pub files: Vec<JavaPreparedJdkFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JavaAnalysisInputAuthority {
    pub schema: String,
    pub policy: String,
    pub adapter_digest: String,
    pub os: String,
    pub arch: String,
    pub os_build: String,
    pub jdk: JavaPreparedJdkAuthority,
    pub sources: Vec<JavaPreparedSourceAuthority>,
    pub classpath: Vec<JavaPreparedClasspathAuthority>,
    pub compiler_options: Vec<String>,
}

#[derive(Debug)]
pub enum JavaPreparedInputsResult {
    Eligible(Box<PreparedJavaAnalysisInputs>),
    Refused(JavaPreparedRefusal),
}

#[derive(Debug)]
struct PreparedInputLease {
    scratch: Option<JavaAnalysisScratch>,
    root: PathBuf,
    runtime: PathBuf,
    sealed: AtomicBool,
}

impl PreparedInputLease {
    fn close(mut self) -> Result<(), ClewError> {
        self.scratch
            .take()
            .map_or(Ok(()), JavaAnalysisScratch::close)
    }
}

#[derive(Debug)]
pub struct PreparedJavaAnalysisInputs {
    pub authority: JavaAnalysisInputAuthority,
    pub java_executable: PathBuf,
    pub analysis_root: PathBuf,
    pub working_dir: PathBuf,
    pub empty_source_path: PathBuf,
    pub source_manifest: PathBuf,
    pub classpath_manifest: PathBuf,
    lease: Arc<PreparedInputLease>,
}

impl PreparedJavaAnalysisInputs {
    pub fn require_sealed(&self) -> Result<(), ClewError> {
        if self.lease.sealed.load(Ordering::Acquire) {
            Ok(())
        } else {
            Err(ClewError::new(
                ErrorCode::UnsupportedProjectConfiguration,
                "prepared Java analysis inputs are not sealed",
            ))
        }
    }

    pub fn authority_digest(&self) -> Result<String, ClewError> {
        canonical::hash(&self.authority).map_err(internal)
    }

    pub(crate) fn duplicate_liveness(&self) -> Result<std::fs::File, ClewError> {
        self.lease
            .scratch
            .as_ref()
            .ok_or_else(|| internal("Java analysis scratch lease is closed"))?
            .duplicate_liveness()
    }
}

#[derive(Debug)]
pub struct JavaAnalysisInputPool {
    lease: Arc<PreparedInputLease>,
    adapter_digest: String,
    os_build: String,
    sealed: bool,
    next_scope: u64,
    classpath: BTreeMap<String, PathBuf>,
    archive_lookup: BTreeMap<String, bool>,
    jdk: Option<(PathBuf, JavaPreparedJdkAuthority)>,
    jdk_source: Option<(PathBuf, PathBuf)>,
    jdk_compiler_version: Option<String>,
}

impl JavaAnalysisInputPool {
    pub fn new(state: &StateAuthority, adapter_digest: String) -> Result<Self, ClewError> {
        if !is_digest(&adapter_digest) {
            return Err(invalid("Java adapter digest is invalid"));
        }
        let scratch = java_analysis_scratch::open(state)?;
        let root = scratch.root().to_owned();
        let runtime = scratch.runtime().to_owned();
        let lease = Arc::new(PreparedInputLease {
            scratch: Some(scratch),
            root,
            runtime,
            sealed: AtomicBool::new(false),
        });
        Ok(Self {
            lease,
            adapter_digest,
            os_build: host_os_build()?,
            sealed: false,
            next_scope: 0,
            classpath: BTreeMap::new(),
            archive_lookup: BTreeMap::new(),
            jdk: None,
            jdk_source: None,
            jdk_compiler_version: None,
        })
    }

    pub fn prepare(
        &mut self,
        repository: &Path,
        model: &JavaOperationalModel,
        source_digests: &BTreeMap<String, String>,
        writable_then_seal: bool,
    ) -> Result<JavaPreparedInputsResult, ClewError> {
        if self.sealed {
            return Err(invalid("prepared Java input pool is already sealed"));
        }
        if let Some(refusal) = refusal_for(model, source_digests, writable_then_seal) {
            return Ok(JavaPreparedInputsResult::Refused(refusal));
        }
        let repository = repository.canonicalize().map_err(io_error)?;
        let (java_executable, jdk_root) =
            match resolve_jdk(&model.java_executable, &model.authority.compiler_version) {
                Ok(value) => value,
                Err(JavaPreparedRefusal::UnqualifiedExecutionImage) => {
                    return Ok(JavaPreparedInputsResult::Refused(
                        JavaPreparedRefusal::UnqualifiedExecutionImage,
                    ));
                }
                Err(_) => {
                    return Ok(JavaPreparedInputsResult::Refused(
                        JavaPreparedRefusal::LegacyInputAuthority,
                    ));
                }
            };
        if let Some((known_root, known_java)) = &self.jdk_source
            && (known_root != &jdk_root || known_java != &java_executable)
        {
            return Ok(JavaPreparedInputsResult::Refused(
                JavaPreparedRefusal::UnqualifiedExecutionImage,
            ));
        }
        if self.jdk.is_none() {
            let destination = self.lease.root.join("jdk");
            let source_authority = jdk_authority(&jdk_root)?;
            copy_jdk_tree(&jdk_root, &destination)?;
            let authority = jdk_authority(&destination)?;
            if source_authority != authority {
                return Err(ClewError::new(
                    ErrorCode::InputMutated,
                    "copied JDK image authority does not match its source",
                ));
            }
            self.jdk = Some((destination, authority));
            self.jdk_source = Some((jdk_root.clone(), java_executable.clone()));
            self.jdk_compiler_version = Some(model.authority.compiler_version.clone());
        }
        let Some((jdk_path, jdk)) = self.jdk.as_ref() else {
            return Err(internal("prepared JDK image was not retained"));
        };
        let scope = self.next_scope;
        self.next_scope += 1;
        let analysis_root = self
            .lease
            .root
            .join("scopes")
            .join(format!("scope-{scope}"));
        let source_root = analysis_root.join("sources");
        fs::create_dir_all(&source_root).map_err(io_error)?;
        let mut sources = Vec::with_capacity(model.authority.source_files.len());
        for path in &model.authority.source_files {
            let expected = source_digests
                .get(path)
                .ok_or_else(|| invalid("prepared Java source authority is incomplete"))?;
            let source = checked_repository_file(&repository, path)?;
            let bytes = read_verified_source(&source, expected)?;
            let destination = source_root.join(path);
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent).map_err(io_error)?;
            }
            fs::write(&destination, bytes).map_err(io_error)?;
            sources.push(JavaPreparedSourceAuthority {
                path: path.clone(),
                digest: expected.clone(),
            });
        }
        let source_manifest = analysis_root.join("sources.txt");
        fs::write(
            &source_manifest,
            manifest_lines(sources.iter().map(|s| s.path.as_str()))?,
        )
        .map_err(io_error)?;

        let mut classpath = Vec::with_capacity(model.authority.classpath.len());
        let mut classpath_paths = Vec::with_capacity(model.classpath_paths.len());
        if model.classpath_paths.len() != model.authority.classpath.len() {
            return Ok(JavaPreparedInputsResult::Refused(
                JavaPreparedRefusal::UnsealedClasspath,
            ));
        }
        for (index, (source, admitted)) in model
            .classpath_paths
            .iter()
            .zip(&model.authority.classpath)
            .enumerate()
        {
            let source = source.canonicalize().map_err(|_| {
                ClewError::new(
                    ErrorCode::UnsupportedProjectConfiguration,
                    "Java classpath entry is unavailable for closed analysis",
                )
            })?;
            verify_classpath_authority(&source, admitted)?;
            let basename = source
                .file_name()
                .and_then(OsStr::to_str)
                .ok_or_else(|| unsupported("Java classpath entry has no UTF-8 basename"))?;
            let key = format!(
                "{}:{}:{}:{}:{}",
                admitted.kind, admitted.logical_name, admitted.digest, admitted.size, basename
            );
            let destination = if let Some(existing) = self.classpath.get(&key) {
                existing.clone()
            } else {
                let base = self
                    .lease
                    .root
                    .join("classpath")
                    .join(digest_component(&admitted.digest)?);
                let destination = if admitted.kind == "FILE" {
                    base.join(source.file_name().unwrap_or_else(|| OsStr::new("entry")))
                } else {
                    base.join(format!("directory-{index}"))
                };
                if admitted.kind == "FILE" {
                    copy_regular_file(&source, &destination)?;
                } else if admitted.kind == "DIRECTORY" {
                    copy_regular_tree(&source, &destination)?;
                } else {
                    return Ok(JavaPreparedInputsResult::Refused(
                        JavaPreparedRefusal::UnsealedClasspath,
                    ));
                }
                verify_classpath_authority(&destination, admitted)?;
                self.classpath.insert(key, destination.clone());
                destination
            };
            if admitted.kind == "FILE" {
                let closed = match self.archive_lookup.get(&admitted.digest) {
                    Some(closed) => *closed,
                    None => {
                        let closed =
                            crate::java_archive_inputs::closed_archive_lookup(&destination)?;
                        self.archive_lookup.insert(admitted.digest.clone(), closed);
                        closed
                    }
                };
                if !closed {
                    return Ok(JavaPreparedInputsResult::Refused(
                        JavaPreparedRefusal::ImplicitOrExternalLookup,
                    ));
                }
            }
            classpath_paths.push(destination.clone());
            classpath.push(JavaPreparedClasspathAuthority {
                authority: admitted.clone(),
            });
        }
        let classpath_manifest = analysis_root.join("classpath.txt");
        if classpath_paths.is_empty() {
            let empty = self.lease.root.join("empty-classpath");
            fs::create_dir_all(&empty).map_err(io_error)?;
            classpath_paths.push(empty);
        }
        let empty_source_path = self.lease.root.join("empty-sourcepath");
        fs::create_dir_all(&empty_source_path).map_err(io_error)?;
        fs::write(
            &classpath_manifest,
            manifest_lines(classpath_paths.iter().map(|path| path.to_string_lossy()))?,
        )
        .map_err(io_error)?;
        let relative_java = java_executable
            .strip_prefix(&jdk_root)
            .map_err(|_| internal("JDK launcher is outside its image"))?;
        let owned_java = jdk_path.join(relative_java);
        let authority = JavaAnalysisInputAuthority {
            schema: JAVA_ANALYSIS_INPUTS_SCHEMA.into(),
            policy: analysis_policy(&model.authority.compiler_version)
                .ok_or_else(|| unsupported("unqualified Java compiler version"))?
                .into(),
            adapter_digest: self.adapter_digest.clone(),
            os: std::env::consts::OS.into(),
            arch: std::env::consts::ARCH.into(),
            os_build: self.os_build.clone(),
            jdk: jdk.clone(),
            sources,
            classpath,
            compiler_options: vec![
                format!("--release={}", model.authority.release),
                "-implicit:none".into(),
                "-proc:none".into(),
                "-sourcepath=<empty>".into(),
                "-classpath=<explicit>".into(),
            ],
        };
        Ok(JavaPreparedInputsResult::Eligible(Box::new(
            PreparedJavaAnalysisInputs {
                authority,
                java_executable: owned_java,
                // The analyzer root is the copied source root so emitted facts keep
                // the original repository-relative source names.
                analysis_root: source_root,
                working_dir: self.lease.runtime.clone(),
                source_manifest,
                classpath_manifest,
                empty_source_path,
                lease: Arc::clone(&self.lease),
            },
        )))
    }

    pub fn seal(&mut self) -> Result<(), ClewError> {
        if self.sealed {
            return Ok(());
        }
        seal_tree(&self.lease.root)?;
        if let Some((path, expected)) = &self.jdk {
            if jdk_authority(path)? != *expected {
                return Err(ClewError::new(
                    ErrorCode::InputMutated,
                    "sealed JDK image differs from admitted authority",
                ));
            }
            let mut command = std::process::Command::new(path.join("bin/java"));
            command
                .arg("--version")
                .env_clear()
                .env("LANG", "C")
                .env("LC_ALL", "C")
                .current_dir(&self.lease.runtime);
            java_analysis_scratch::attach_child_lease(
                &mut command,
                self.lease
                    .scratch
                    .as_ref()
                    .ok_or_else(|| internal("Java analysis scratch lease is closed"))?
                    .duplicate_liveness()?,
            )?;
            let output = command.output().map_err(io_error)?;
            let version_output = String::from_utf8(output.stdout).map_err(internal)?;
            let observed = version_output
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1));
            let expected_version = self
                .jdk_compiler_version
                .as_deref()
                .and_then(|version| version.strip_prefix("javac "));
            if !output.status.success() || observed.is_none() || observed != expected_version {
                return Err(ClewError::new(
                    ErrorCode::InputMutated,
                    "owned Java execution image differs from admitted compiler version",
                ));
            }
        }
        self.sealed = true;
        self.lease.sealed.store(true, Ordering::Release);
        Ok(())
    }

    pub fn close(self) -> Result<(), ClewError> {
        let lease = Arc::try_unwrap(self.lease)
            .map_err(|_| invalid("prepared Java input pool still has live prepared scopes"))?;
        lease.close()
    }
}

fn refusal_for(
    model: &JavaOperationalModel,
    source_digests: &BTreeMap<String, String>,
    writable_then_seal: bool,
) -> Option<JavaPreparedRefusal> {
    if writable_then_seal {
        return Some(JavaPreparedRefusal::WritableTransform);
    }
    if model.authority.build_system != JavaBuildSystem::Maven {
        return Some(JavaPreparedRefusal::LegacyInputAuthority);
    }
    if !model.authority.annotation_processors.is_empty()
        || !model.authority.annotation_processor_paths.is_empty()
        || model
            .authority
            .compiler_options
            .iter()
            .any(|option| option.starts_with("-A"))
    {
        return Some(JavaPreparedRefusal::ProcessorInputsPresent);
    }
    if model
        .authority
        .source_files
        .iter()
        .any(|path| Path::new(path).file_name() == Some(OsStr::new("module-info.java")))
    {
        return Some(JavaPreparedRefusal::UnsupportedAnalyzerOptions);
    }
    if !qualified_release(&model.authority.compiler_version, model.authority.release) {
        return Some(JavaPreparedRefusal::UnqualifiedExecutionImage);
    }
    if model.authority.compiler_options.iter().any(|option| {
        option != &format!("--release={}", model.authority.release) && option != "-implicit:none"
    }) {
        return Some(JavaPreparedRefusal::UnsupportedAnalyzerOptions);
    }
    if source_digests.len() != model.authority.source_files.len()
        || model
            .authority
            .source_files
            .iter()
            .any(|path| !source_digests.contains_key(path))
    {
        return Some(JavaPreparedRefusal::LegacyInputAuthority);
    }
    if !qualified_platform(
        &model.authority.compiler_version,
        std::env::consts::OS,
        std::env::consts::ARCH,
    ) {
        return Some(JavaPreparedRefusal::UnqualifiedExecutionImage);
    }
    None
}

fn host_os_build() -> Result<String, ClewError> {
    if std::env::consts::OS != "macos" {
        return Ok("UNQUALIFIED_PLATFORM".into());
    }
    let output = std::process::Command::new("/usr/bin/sw_vers")
        .arg("-buildVersion")
        .env_clear()
        .output()
        .map_err(io_error)?;
    let value = String::from_utf8(output.stdout).map_err(internal)?;
    let value = value.trim();
    if !output.status.success()
        || value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'.')
    {
        return Err(unsupported(
            "macOS execution build authority is unavailable",
        ));
    }
    Ok(value.to_owned())
}

fn resolve_jdk(
    java: &Path,
    compiler_version: &str,
) -> Result<(PathBuf, PathBuf), JavaPreparedRefusal> {
    let executable = if java.is_absolute() || java.components().count() > 1 {
        java.to_owned()
    } else if let Some(home) = std::env::var_os("JAVA_HOME") {
        PathBuf::from(home).join("bin").join(java)
    } else {
        let path =
            std::env::var_os("PATH").ok_or(JavaPreparedRefusal::UnqualifiedExecutionImage)?;
        std::env::split_paths(&path)
            .map(|entry| entry.join(java))
            .find(|entry| entry.is_file())
            .ok_or(JavaPreparedRefusal::UnqualifiedExecutionImage)?
    };
    let executable = executable
        .canonicalize()
        .map_err(|_| JavaPreparedRefusal::UnqualifiedExecutionImage)?;
    if !executable.is_file() || executable.file_name() != Some(OsStr::new("java")) {
        return Err(JavaPreparedRefusal::UnqualifiedExecutionImage);
    }
    let root = executable
        .parent()
        .and_then(Path::parent)
        .ok_or(JavaPreparedRefusal::UnqualifiedExecutionImage)?
        .canonicalize()
        .map_err(|_| JavaPreparedRefusal::UnqualifiedExecutionImage)?;
    let expected = root
        .join("bin/java")
        .canonicalize()
        .map_err(|_| JavaPreparedRefusal::UnqualifiedExecutionImage)?;
    let javac = root
        .join("bin/javac")
        .canonicalize()
        .map_err(|_| JavaPreparedRefusal::UnqualifiedExecutionImage)?;
    if expected != executable
        || !javac.starts_with(&root)
        || javac.file_name() != Some(OsStr::new("javac"))
        || !root.join("lib/modules").is_file()
        || !native_macos_launcher(&executable)
        || !native_macos_launcher(&javac)
        || !qualified_jdk_release(&root, compiler_version)
    {
        return Err(JavaPreparedRefusal::UnqualifiedExecutionImage);
    }
    Ok((executable, root))
}

fn native_macos_launcher(path: &Path) -> bool {
    let mut header = [0u8; 8];
    let Ok(mut file) = fs::File::open(path) else {
        return false;
    };
    if file.read_exact(&mut header).is_err() || header[..4] != [0xcf, 0xfa, 0xed, 0xfe] {
        return false;
    }
    let expected_cpu = match std::env::consts::ARCH {
        "aarch64" => 0x0100_000cu32,
        "x86_64" => 0x0100_0007u32,
        _ => return false,
    };
    u32::from_le_bytes(header[4..8].try_into().unwrap_or_default()) == expected_cpu
}

fn compiler_major(compiler_version: &str) -> Option<u16> {
    let version = compiler_version.strip_prefix("javac ")?;
    if version.is_empty()
        || !version.bytes().all(|byte| {
            byte.is_ascii_digit()
                || matches!(byte, b'.' | b'+' | b'-')
                || byte.is_ascii_alphabetic()
        })
    {
        return None;
    }
    version.split(['.', '+', '-']).next()?.parse().ok()
}

fn analysis_policy(compiler_version: &str) -> Option<&'static str> {
    match compiler_major(compiler_version)? {
        17 => Some(JAVA_ANALYSIS_POLICY_JDK17),
        21 => Some(JAVA_ANALYSIS_POLICY),
        _ => None,
    }
}

fn qualified_platform(compiler_version: &str, os: &str, arch: &str) -> bool {
    os == "macos"
        && match compiler_major(compiler_version) {
            Some(17) => arch == "aarch64",
            Some(21) => matches!(arch, "aarch64" | "x86_64"),
            _ => false,
        }
}

fn qualified_release(compiler_version: &str, release: u16) -> bool {
    matches!(
        (compiler_major(compiler_version), release),
        (Some(17), 17) | (Some(21), 17 | 21)
    )
}

fn qualified_jdk_release(root: &Path, compiler_version: &str) -> bool {
    let Ok(file) = fs::File::open(root.join("release")) else {
        return false;
    };
    let mut release = String::new();
    if file.take(65_537).read_to_string(&mut release).is_err() || release.len() > 65_536 {
        return false;
    }
    let values = release
        .lines()
        .filter_map(|line| line.strip_prefix("JAVA_VERSION=\""))
        .collect::<Vec<_>>();
    values.len() == 1
        && values[0].strip_suffix('"').is_some_and(|version| {
            let release_compiler = format!("javac {version}");
            analysis_policy(&release_compiler).is_some()
                && (compiler_major(&release_compiler) != Some(21) || version.starts_with("21."))
                && compiler_major(&release_compiler) == compiler_major(compiler_version)
        })
        && release.lines().any(|line| {
            line.starts_with("MODULES=\"")
                && line
                    .split_whitespace()
                    .any(|module| module == "jdk.compiler")
        })
}

fn copy_jdk_tree(source: &Path, destination: &Path) -> Result<(), ClewError> {
    copy_tree(source, destination, Some(source))
}

fn copy_regular_tree(source: &Path, destination: &Path) -> Result<(), ClewError> {
    copy_tree(source, destination, None)
}

fn copy_tree(source: &Path, destination: &Path, link_root: Option<&Path>) -> Result<(), ClewError> {
    for entry in walkdir::WalkDir::new(source).follow_links(false) {
        let entry = entry.map_err(internal)?;
        let relative = entry.path().strip_prefix(source).map_err(internal)?;
        let target = destination.join(relative);
        let metadata = fs::symlink_metadata(entry.path()).map_err(io_error)?;
        if metadata.file_type().is_symlink() {
            let Some(link_root) = link_root else {
                return Err(unsupported("closed Java classpath contains a symlink"));
            };
            let link = fs::read_link(entry.path()).map_err(io_error)?;
            if link.is_absolute() {
                return Err(unsupported("JDK image contains an absolute symlink"));
            }
            let resolved = entry
                .path()
                .parent()
                .unwrap_or(source)
                .join(&link)
                .canonicalize()
                .map_err(|_| unsupported("JDK image contains an unresolved symlink"))?;
            if !resolved.starts_with(link_root) {
                return Err(unsupported("JDK image contains an escaping symlink"));
            }
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(io_error)?;
            }
            #[cfg(unix)]
            std::os::unix::fs::symlink(&link, &target).map_err(io_error)?;
            // macOS derives new link permissions from umask. Preserve the
            // link itself, without following it or weakening image equality.
            #[cfg(target_os = "macos")]
            preserve_symlink_mode(&target, &metadata)?;
            #[cfg(not(unix))]
            return Err(unsupported(
                "closed Java JDK image requires Unix symlink support",
            ));
        } else if metadata.is_dir() {
            fs::create_dir_all(&target).map_err(io_error)?;
            fs::set_permissions(&target, metadata.permissions()).map_err(io_error)?;
        } else if metadata.is_file() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(io_error)?;
            }
            fs::copy(entry.path(), &target).map_err(io_error)?;
            fs::set_permissions(&target, metadata.permissions()).map_err(io_error)?;
        } else {
            return Err(unsupported("closed Java input contains a special file"));
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn preserve_symlink_mode(target: &Path, source: &fs::Metadata) -> Result<(), ClewError> {
    use std::os::unix::fs::PermissionsExt;
    let path = std::ffi::CString::new(target.as_os_str().as_encoded_bytes()).map_err(internal)?;
    let mode = (source.permissions().mode() & 0o777) as libc::mode_t;
    if unsafe {
        libc::fchmodat(
            libc::AT_FDCWD,
            path.as_ptr(),
            mode,
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } != 0
    {
        return Err(io_error(std::io::Error::last_os_error()));
    }
    Ok(())
}

fn copy_regular_file(source: &Path, destination: &Path) -> Result<(), ClewError> {
    let metadata = fs::symlink_metadata(source).map_err(io_error)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(unsupported(
            "closed Java classpath entry is not a regular file",
        ));
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(io_error)?;
    }
    fs::copy(source, destination).map_err(io_error)?;
    fs::set_permissions(destination, metadata.permissions()).map_err(io_error)
}

fn jdk_authority(root: &Path) -> Result<JavaPreparedJdkAuthority, ClewError> {
    let mut files = Vec::new();
    let mut bytes = 0u64;
    for entry in walkdir::WalkDir::new(root).follow_links(false) {
        let entry = entry.map_err(internal)?;
        let relative = entry.path().strip_prefix(root).map_err(internal)?;
        if relative.as_os_str().is_empty() {
            continue;
        }
        if files.len() >= MAX_JDK_FILES {
            return Err(resource("JDK image exceeds its file budget"));
        }
        let metadata = fs::symlink_metadata(entry.path()).map_err(io_error)?;
        let (kind, size, digest, link_target) = if metadata.file_type().is_symlink() {
            (
                "SYMLINK".into(),
                0,
                String::new(),
                Some(
                    fs::read_link(entry.path())
                        .map_err(io_error)?
                        .to_string_lossy()
                        .into_owned(),
                ),
            )
        } else if metadata.is_dir() {
            ("DIRECTORY".into(), 0, String::new(), None)
        } else if metadata.is_file() {
            let size = metadata.len();
            bytes = bytes
                .checked_add(size)
                .ok_or_else(|| resource("JDK image size overflows"))?;
            if bytes > MAX_JDK_BYTES {
                return Err(resource("JDK image exceeds its byte budget"));
            }
            let digest = hash_file(entry.path())?;
            ("FILE".into(), size, digest, None)
        } else {
            return Err(unsupported("JDK image contains a special file"));
        };
        files.push(JavaPreparedJdkFile {
            path: relative.to_string_lossy().replace('\\', "/"),
            kind,
            mode: file_mode(&metadata),
            size,
            digest,
            link_target,
        });
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    let image_digest = canonical::hash(&files).map_err(internal)?;
    Ok(JavaPreparedJdkAuthority {
        image_digest,
        files,
    })
}

fn hash_file(path: &Path) -> Result<String, ClewError> {
    let mut file = fs::File::open(path).map_err(io_error)?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(io_error)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("sha256:{}", hex::encode(digest.finalize())))
}

fn read_verified_source(path: &Path, expected: &str) -> Result<Vec<u8>, ClewError> {
    let bytes = fs::read(path).map_err(io_error)?;
    if canonical::hash_bytes(&bytes) != expected {
        return Err(ClewError::new(
            ErrorCode::InputMutated,
            "Java source bytes changed after model admission",
        ));
    }
    Ok(bytes)
}

fn verify_classpath_authority(
    path: &Path,
    expected: &JavaClasspathAuthority,
) -> Result<(), ClewError> {
    if crate::java_project_model::classpath_authority(path)? != *expected {
        return Err(ClewError::new(
            ErrorCode::InputMutated,
            "Java classpath bytes differ from admitted authority",
        ));
    }
    Ok(())
}

fn checked_repository_file(repository: &Path, relative: &str) -> Result<PathBuf, ClewError> {
    let path = Path::new(relative);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(invalid(
            "Java source authority path is not repository-relative",
        ));
    }
    let candidate = repository.join(path);
    let canonical = candidate.canonicalize().map_err(io_error)?;
    if !canonical.starts_with(repository) {
        return Err(unsupported("Java source authority escapes the repository"));
    }
    let metadata = fs::symlink_metadata(&candidate).map_err(io_error)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(unsupported("Java source authority is not a regular file"));
    }
    Ok(canonical)
}

fn manifest_lines(values: impl IntoIterator<Item = impl AsRef<str>>) -> Result<String, ClewError> {
    let mut output = String::new();
    for value in values {
        let value = value.as_ref();
        if value.is_empty() || value.chars().any(|ch| matches!(ch, '\n' | '\r' | '\0')) {
            return Err(invalid(
                "Java input path cannot be represented in a line manifest",
            ));
        }
        output.push_str(value);
        output.push('\n');
    }
    Ok(output)
}

fn digest_component(value: &str) -> Result<&str, ClewError> {
    let value = value
        .strip_prefix("sha256:")
        .ok_or_else(|| invalid("Java input digest prefix is invalid"))?;
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(invalid("Java input digest is invalid"));
    }
    Ok(value)
}

fn is_digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|value| {
        value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}

#[cfg(unix)]
fn file_mode(metadata: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    if metadata.file_type().is_symlink() {
        metadata.permissions().mode() & 0o777
    } else if metadata.is_dir() || metadata.permissions().mode() & 0o111 != 0 {
        0o500
    } else {
        0o400
    }
}

#[cfg(not(unix))]
fn file_mode(_metadata: &fs::Metadata) -> u32 {
    0
}

fn invalid(message: &str) -> ClewError {
    ClewError::new(ErrorCode::InvalidInput, message)
}

fn unsupported(message: &str) -> ClewError {
    ClewError::new(ErrorCode::UnsupportedProjectConfiguration, message)
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
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    fn no_processor_model() -> JavaOperationalModel {
        use crate::java_project_model::{JAVA_MODEL_SCHEMA, JavaProjectModel};
        JavaOperationalModel {
            authority: JavaProjectModel {
                schema: JAVA_MODEL_SCHEMA.into(),
                model_digest: String::new(),
                build_system: JavaBuildSystem::Maven,
                compilation: ":/main".into(),
                source_files: vec!["src/Main.java".into()],
                classpath: Vec::new(),
                release: 17,
                compiler_version: "javac 17.0.20.1".into(),
                compiler_options: vec!["--release=17".into(), "-implicit:none".into()],
                annotation_processors: Vec::new(),
                annotation_processor_paths: Vec::new(),
                boundaries: Vec::new(),
            },
            source_paths: vec![PathBuf::from("src/Main.java")],
            classpath_paths: Vec::new(),
            annotation_processor_paths: Vec::new(),
            java_executable: PathBuf::from("java"),
        }
    }

    #[test]
    fn qualified_compiler_pairs_keep_jdk21_identity_and_reject_lookalikes() {
        for (compiler, release, expected) in [
            ("javac 17", 17, true),
            ("javac 17.0.20.1", 17, true),
            ("javac 17.0.20.1", 21, false),
            ("javac 21.0.9", 17, true),
            ("javac 21.0.9", 21, true),
            ("javac 21.0.9", 11, false),
            ("javac 25.0.1", 17, false),
            ("javac 210", 21, false),
            ("javac 170", 17, false),
            ("javac 17garbage", 17, false),
            ("javac 17\nextra", 17, false),
            ("17.0.20.1", 17, false),
        ] {
            assert_eq!(
                qualified_release(compiler, release),
                expected,
                "{compiler}/{release}"
            );
        }
        assert_eq!(analysis_policy("javac 21.0.9"), Some(JAVA_ANALYSIS_POLICY));
        assert_eq!(
            analysis_policy("javac 17.0.20.1"),
            Some(JAVA_ANALYSIS_POLICY_JDK17)
        );
        assert_ne!(JAVA_ANALYSIS_POLICY_JDK17, JAVA_ANALYSIS_POLICY);
    }

    #[test]
    fn jdk17_requires_qualified_arm64_host_without_narrowing_jdk21() {
        for (compiler, os, arch, expected) in [
            ("javac 17.0.20.1", "macos", "aarch64", true),
            ("javac 17.0.20.1", "macos", "x86_64", false),
            ("javac 17.0.20.1", "linux", "aarch64", false),
            ("javac 21.0.9", "macos", "aarch64", true),
            ("javac 21.0.9", "macos", "x86_64", true),
            ("javac 21.0.9", "linux", "x86_64", false),
            ("javac 25.0.1", "macos", "aarch64", false),
        ] {
            assert_eq!(
                qualified_platform(compiler, os, arch),
                expected,
                "{compiler}/{os}/{arch}"
            );
        }
    }

    #[test]
    fn jdk17_admission_keeps_processor_writable_and_source_guards() {
        let model = no_processor_model();
        let digests = BTreeMap::from([(
            "src/Main.java".into(),
            canonical::hash_bytes(b"class Main {}"),
        )]);
        let platform_refusal =
            if std::env::consts::OS == "macos" && std::env::consts::ARCH == "aarch64" {
                None
            } else {
                Some(JavaPreparedRefusal::UnqualifiedExecutionImage)
            };
        assert_eq!(refusal_for(&model, &digests, false), platform_refusal);
        assert_eq!(
            refusal_for(&model, &digests, true),
            Some(JavaPreparedRefusal::WritableTransform)
        );
        assert_eq!(
            refusal_for(&model, &BTreeMap::new(), false),
            Some(JavaPreparedRefusal::LegacyInputAuthority)
        );
        for processor_kind in 0..3 {
            let mut ap = model.clone();
            match processor_kind {
                0 => ap
                    .authority
                    .annotation_processors
                    .push("example.Processor".into()),
                1 => ap
                    .authority
                    .annotation_processor_paths
                    .push(JavaClasspathAuthority {
                        logical_name: "processor.jar".into(),
                        digest: canonical::hash_bytes(b"processor"),
                        size: 9,
                        kind: "JAR".into(),
                    }),
                _ => ap
                    .authority
                    .compiler_options
                    .push("-Aexternal=ambient".into()),
            }
            assert_eq!(
                refusal_for(&ap, &digests, false),
                Some(JavaPreparedRefusal::ProcessorInputsPresent)
            );
        }
        let mut module = model.clone();
        module
            .authority
            .source_files
            .push("src/module-info.java".into());
        assert_eq!(
            refusal_for(&module, &digests, false),
            Some(JavaPreparedRefusal::UnsupportedAnalyzerOptions)
        );
        let mut external = model;
        external
            .authority
            .compiler_options
            .push("--source-path=/outside".into());
        assert_eq!(
            refusal_for(&external, &digests, false),
            Some(JavaPreparedRefusal::UnsupportedAnalyzerOptions)
        );
    }

    #[test]
    fn jdk_release_requires_matching_qualified_major_and_compiler_module() {
        let root = tempfile::tempdir().unwrap();
        for (release, compiler, expected) in [
            (
                "JAVA_VERSION=\"21\"\nMODULES=\"java.base jdk.compiler jdk.zipfs\"\n",
                "javac 21",
                false,
            ),
            (
                "JAVA_VERSION=\"17.0.20.1\"\nMODULES=\"java.base jdk.compiler jdk.zipfs\"\n",
                "javac 17.0.20.1",
                true,
            ),
            (
                "JAVA_VERSION=\"21.0.9\"\nMODULES=\"java.base jdk.compiler jdk.zipfs\"\n",
                "javac 21.0.9",
                true,
            ),
            (
                "JAVA_VERSION=\"17.0.20.1\"\nMODULES=\"java.base jdk.compiler jdk.zipfs\"\n",
                "javac 21.0.9",
                false,
            ),
            (
                "JAVA_VERSION=\"25.0.1\"\nMODULES=\"java.base jdk.compiler jdk.zipfs\"\n",
                "javac 25.0.1",
                false,
            ),
            (
                "JAVA_VERSION=\"17.0.20.1\"\nMODULES=\"java.base\"\n",
                "javac 17.0.20.1",
                false,
            ),
            (
                "JAVA_VERSION=\"17.0.20.1\"\nJAVA_VERSION=\"21.0.9\"\nMODULES=\"java.base jdk.compiler jdk.zipfs\"\n",
                "javac 17.0.20.1",
                false,
            ),
        ] {
            fs::write(root.path().join("release"), release).unwrap();
            assert_eq!(
                qualified_jdk_release(root.path(), compiler),
                expected,
                "{release}/{compiler}"
            );
        }
    }

    #[test]
    fn prepared_refusal_codes_are_stable() {
        assert_eq!(
            JavaPreparedRefusal::UnsupportedAnalyzerOptions.code(),
            "UNSUPPORTED_ANALYZER_OPTIONS_OR_MODULE_MODE"
        );
        assert_eq!(
            JavaPreparedRefusal::UnqualifiedExecutionImage.code(),
            "UNQUALIFIED_EXECUTION_IMAGE"
        );
        assert_eq!(
            JavaPreparedRefusal::ImplicitOrExternalLookup.code(),
            "IMPLICIT_OR_EXTERNAL_LOOKUP"
        );
    }

    #[test]
    fn digest_component_rejects_path_injection() {
        assert!(digest_component("sha256:../classpath").is_err());
        assert!(digest_component("sha256:short").is_err());
        assert!(digest_component(&format!("sha256:{}", "a".repeat(64))).is_ok());
    }
    #[test]
    fn changed_source_and_classpath_stop_instead_of_falling_back() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("Main.java");
        fs::write(&source, b"class Main {}").unwrap();
        let expected = canonical::hash_bytes(b"class Main {}");
        assert!(read_verified_source(&source, &expected).is_ok());
        fs::write(&source, b"class Main { int changed; }").unwrap();
        assert_eq!(
            read_verified_source(&source, &expected).unwrap_err().code,
            ErrorCode::InputMutated
        );
        let classes = root.path().join("classes");
        fs::create_dir(&classes).unwrap();
        fs::write(classes.join("Main.class"), b"admitted").unwrap();
        let admitted = crate::java_project_model::classpath_authority(&classes).unwrap();
        verify_classpath_authority(&classes, &admitted).unwrap();
        fs::write(classes.join("Main.class"), b"changed").unwrap();
        assert_eq!(
            verify_classpath_authority(&classes, &admitted)
                .unwrap_err()
                .code,
            ErrorCode::InputMutated
        );
    }

    #[test]
    #[cfg(unix)]
    fn jdk_image_rejects_escaping_links_and_normalizes_sealed_modes() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("jdk");
        let copy = root.path().join("copy");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("file"), b"jdk bytes").unwrap();
        let authority = jdk_authority(&source).unwrap();
        seal_tree(&source).unwrap();
        assert_eq!(jdk_authority(&source).unwrap(), authority);
        // Restore writability only for this test fixture before adding a link;
        // production cleanup is descriptor-relative and never chmods by path.
        #[cfg(unix)]
        fs::set_permissions(&source, fs::Permissions::from_mode(0o700)).unwrap();
        symlink("../outside", source.join("escape")).unwrap();
        fs::write(root.path().join("outside"), b"external").unwrap();
        assert_eq!(
            copy_jdk_tree(&source, &copy).unwrap_err().code,
            ErrorCode::UnsupportedProjectConfiguration
        );
    }
    #[test]
    #[cfg(target_os = "macos")]
    fn jdk_copy_preserves_symlink_modes_under_restricted_umask() {
        const CHILD_ROOT: &str = "CODECLEW_TEST_JDK_LINK_MODE_ROOT";
        if let Some(root) = std::env::var_os(CHILD_ROOT) {
            // Process-local mutation: no other test in the parent sees this umask.
            unsafe {
                libc::umask(0o077);
            }
            let root = PathBuf::from(root);
            let source = root.join("source");
            let copy = root.join("copy");
            let before = jdk_authority(&source).unwrap();
            copy_jdk_tree(&source, &copy).unwrap();
            assert_eq!(jdk_authority(&copy).unwrap(), before);
            assert_eq!(jdk_authority(&source).unwrap(), before);
            assert_eq!(
                fs::read(copy.join("link")).unwrap(),
                b"execution image fixture"
            );
            assert_eq!(
                fs::metadata(copy.join("file"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o640
            );
            return;
        }
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("file"), b"execution image fixture").unwrap();
        fs::set_permissions(source.join("file"), fs::Permissions::from_mode(0o640)).unwrap();
        symlink("file", source.join("link")).unwrap();
        let path =
            std::ffi::CString::new(source.join("link").as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(
            unsafe {
                libc::fchmodat(
                    libc::AT_FDCWD,
                    path.as_ptr(),
                    0o755,
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            },
            0
        );
        assert_eq!(
            fs::symlink_metadata(source.join("link"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o755
        );
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "java_analysis_inputs::tests::jdk_copy_preserves_symlink_modes_under_restricted_umask", "--nocapture"])
            .env(CHILD_ROOT, root.path()).output().unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn closed_manifests_reject_line_injection_and_jdk_scripts() {
        for value in ["file\nother", "file\rother", "file\0other", ""] {
            assert!(manifest_lines([value]).is_err());
        }
        assert_eq!(
            manifest_lines(["src/Main.java"]).unwrap(),
            "src/Main.java\n"
        );
        let root = tempfile::tempdir().unwrap();
        let script = root.path().join("java");
        fs::write(&script, b"#!/bin/sh\nexec /outside/java \"$@\"\n").unwrap();
        assert!(!native_macos_launcher(&script));
    }
}
