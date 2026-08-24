use crate::canonical;
use crate::error::{ClewError, ErrorCode};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};

pub const CARGO_MODEL_SCHEMA: &str = "codeclew-cargo-project-model/1.0";
const SELECTOR_PREFIX: &str = "cargo:";
const MAX_METADATA_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RustCompilationSelector {
    pub manifest: String,
    pub package: String,
    pub target_kind: String,
    pub target_name: String,
}

impl RustCompilationSelector {
    pub fn parse(value: &str) -> Result<Self, ClewError> {
        if value.len() > 512 || !value.starts_with(SELECTOR_PREFIX) {
            return Err(invalid(
                "Rust compilation selector prefix or size is invalid",
            ));
        }
        let parts = value[SELECTOR_PREFIX.len()..]
            .split('#')
            .collect::<Vec<_>>();
        if parts.len() != 4
            || !safe_relative_manifest(parts[0])
            || parts[1..].iter().any(|part| !safe_component(part))
        {
            return Err(invalid("Rust compilation selector is invalid"));
        }
        Ok(Self {
            manifest: parts[0].into(),
            package: parts[1].into(),
            target_kind: parts[2].into(),
            target_name: parts[3].into(),
        })
    }

    pub fn canonical(&self) -> String {
        format!(
            "{SELECTOR_PREFIX}{}#{}#{}#{}",
            self.manifest, self.package, self.target_kind, self.target_name
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CargoModelInput {
    pub path: String,
    pub digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CargoTargetModel {
    pub selector: RustCompilationSelector,
    pub source_path: String,
    pub edition: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CargoProjectModel {
    pub schema: String,
    pub model_digest: String,
    pub cargo_version: String,
    pub rustc_version: String,
    pub workspace_manifest: String,
    pub inputs: Vec<CargoModelInput>,
    pub targets: Vec<CargoTargetModel>,
}

impl CargoProjectModel {
    pub fn verify(&self) -> Result<(), ClewError> {
        if self.schema != CARGO_MODEL_SCHEMA
            || self.workspace_manifest != "Cargo.toml"
            || self.cargo_version.is_empty()
            || self.rustc_version.is_empty()
            || self.inputs.is_empty()
            || self.targets.is_empty()
            || self
                .inputs
                .windows(2)
                .any(|pair| pair[0].path >= pair[1].path)
            || self
                .targets
                .windows(2)
                .any(|pair| pair[0].selector >= pair[1].selector)
            || self
                .inputs
                .iter()
                .any(|input| !safe_relative_input(&input.path) || !canonical_digest(&input.digest))
            || self.targets.iter().any(|target| {
                RustCompilationSelector::parse(&target.selector.canonical()).is_err()
                    || !safe_relative_input(&target.source_path)
                    || !target.source_path.ends_with(".rs")
                    || !safe_component(&target.edition)
            })
        {
            return Err(unsupported("Cargo project model authority is invalid"));
        }
        let mut unsigned = self.clone();
        unsigned.model_digest.clear();
        if self.model_digest != canonical::hash(&unsigned).map_err(internal)? {
            return Err(unsupported("Cargo project model digest is invalid"));
        }
        Ok(())
    }
}

pub fn extract_cargo_model(
    repository: &Path,
    requested: &[String],
) -> Result<CargoProjectModel, ClewError> {
    let repository = repository
        .canonicalize()
        .map_err(|_| unsupported("Rust repository cannot be resolved"))?;
    let workspace_manifest = repository.join("Cargo.toml");
    require_regular_file(&workspace_manifest, "workspace Cargo.toml")?;
    require_regular_file(&repository.join("Cargo.lock"), "workspace Cargo.lock")?;
    let metadata = run_bounded(
        Command::new("cargo")
            .args([
                "metadata",
                "--format-version",
                "1",
                "--no-deps",
                "--manifest-path",
            ])
            .arg(&workspace_manifest)
            .current_dir(&repository),
        "Cargo metadata extraction failed",
    )?;
    let metadata: Value = serde_json::from_slice(&metadata)
        .map_err(|_| unsupported("Cargo metadata response is invalid"))?;
    let workspace_root = normalized_relative_path(
        &repository,
        required_path(&metadata, "workspace_root")?,
        true,
    )?;
    let workspace_manifest_relative = if workspace_root.is_empty() {
        "Cargo.toml".to_owned()
    } else {
        format!("{workspace_root}/Cargo.toml")
    };
    if workspace_manifest_relative != "Cargo.toml" {
        return Err(unsupported(
            "Cargo workspace root must be the selected repository root",
        ));
    }
    let requested_count = requested.len();
    let requested = requested
        .iter()
        .map(|value| RustCompilationSelector::parse(value))
        .collect::<Result<BTreeSet<_>, _>>()?;
    if requested.is_empty() || requested.len() != requested_count {
        return Err(invalid(
            "Rust compilation selector set is empty or duplicated",
        ));
    }
    let packages = metadata
        .get("packages")
        .and_then(Value::as_array)
        .ok_or_else(|| unsupported("Cargo metadata has no package set"))?;
    let mut observed = BTreeSet::new();
    let mut targets = Vec::new();
    let mut manifest_paths = BTreeSet::new();
    for package in packages {
        let package_name = required_string(package, "name")?;
        let manifest =
            normalized_relative_path(&repository, required_path(package, "manifest_path")?, false)?;
        if !safe_relative_manifest(&manifest) {
            return Err(unsupported("Cargo package manifest path is unsafe"));
        }
        manifest_paths.insert(manifest.clone());
        let package_targets = package
            .get("targets")
            .and_then(Value::as_array)
            .ok_or_else(|| unsupported("Cargo package has no target set"))?;
        for target in package_targets {
            let target_name = required_string(target, "name")?;
            let edition = required_string(target, "edition")?;
            let kinds = target
                .get("kind")
                .and_then(Value::as_array)
                .ok_or_else(|| unsupported("Cargo target has no kind"))?;
            let source_path =
                normalized_relative_path(&repository, required_path(target, "src_path")?, false)?;
            for kind in kinds {
                let kind = kind
                    .as_str()
                    .ok_or_else(|| unsupported("Cargo target kind is invalid"))?;
                let selector = RustCompilationSelector {
                    manifest: manifest.clone(),
                    package: package_name.into(),
                    target_kind: kind.into(),
                    target_name: target_name.into(),
                };
                if requested.contains(&selector) {
                    observed.insert(selector.clone());
                    targets.push(CargoTargetModel {
                        selector,
                        source_path: source_path.clone(),
                        edition: edition.into(),
                    });
                }
            }
        }
    }
    if observed != requested {
        return Err(unsupported(
            "requested Rust compilation does not match an exact Cargo target",
        ));
    }
    targets.sort_by(|left, right| left.selector.cmp(&right.selector));
    let mut input_paths = manifest_paths;
    for candidate in [
        "Cargo.lock",
        ".cargo/config",
        ".cargo/config.toml",
        "rust-toolchain",
        "rust-toolchain.toml",
    ] {
        if repository.join(candidate).exists() {
            input_paths.insert(candidate.into());
        }
    }
    let mut inputs = input_paths
        .into_iter()
        .map(|path| {
            let absolute = repository.join(&path);
            require_regular_file(&absolute, "Cargo model input")?;
            let bytes =
                fs::read(absolute).map_err(|_| unsupported("Cargo model input cannot be read"))?;
            Ok(CargoModelInput {
                path,
                digest: canonical::hash_bytes(&bytes),
            })
        })
        .collect::<Result<Vec<_>, ClewError>>()?;
    inputs.sort_by(|left, right| left.path.cmp(&right.path));
    let cargo_version = command_version(
        &repository,
        "cargo",
        &["--version"],
        "Cargo version query failed",
    )?;
    let rustc_version =
        command_version(&repository, "rustc", &["-vV"], "Rust version query failed")?;
    let mut model = CargoProjectModel {
        schema: CARGO_MODEL_SCHEMA.into(),
        model_digest: String::new(),
        cargo_version,
        rustc_version,
        workspace_manifest: "Cargo.toml".into(),
        inputs,
        targets,
    };
    model.model_digest = canonical::hash(&model).map_err(internal)?;
    model.verify()?;
    Ok(model)
}

fn command_version(
    repository: &Path,
    program: &str,
    arguments: &[&str],
    message: &str,
) -> Result<String, ClewError> {
    let bytes = run_bounded(
        Command::new(program)
            .args(arguments)
            .current_dir(repository),
        message,
    )?;
    let value = std::str::from_utf8(&bytes)
        .map_err(|_| unsupported(message))?
        .trim();
    if value.is_empty() || value.len() > 4096 || value.contains('\0') {
        return Err(unsupported(message));
    }
    Ok(value.into())
}

fn run_bounded(command: &mut Command, message: &str) -> Result<Vec<u8>, ClewError> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| unsupported(message))?;
    let mut stdout = child.stdout.take().ok_or_else(|| unsupported(message))?;
    let mut bytes = Vec::new();
    stdout
        .by_ref()
        .take((MAX_METADATA_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| unsupported(message))?;
    if bytes.len() > MAX_METADATA_BYTES {
        let _ = child.kill();
        let _ = child.wait();
        return Err(unsupported(message));
    }
    let status = child.wait().map_err(|_| unsupported(message))?;
    if !status.success() {
        return Err(unsupported(message));
    }
    Ok(bytes)
}

fn normalized_relative_path(
    repository: &Path,
    raw: &Path,
    allow_root: bool,
) -> Result<String, ClewError> {
    if !raw.is_absolute() {
        return Err(unsupported("Cargo metadata path is not absolute"));
    }
    let lexical_relative = raw
        .strip_prefix(repository)
        .map_err(|_| unsupported("Cargo metadata path escapes the repository"))?;
    let mut observed = repository.to_path_buf();
    for component in lexical_relative.components() {
        observed.push(component);
        let metadata = fs::symlink_metadata(&observed)
            .map_err(|_| unsupported("Cargo metadata path cannot be inspected"))?;
        if metadata.file_type().is_symlink() {
            return Err(unsupported("Cargo metadata path contains a symlink"));
        }
    }
    let resolved = raw
        .canonicalize()
        .map_err(|_| unsupported("Cargo metadata path cannot be resolved"))?;
    let relative = resolved
        .strip_prefix(repository)
        .map_err(|_| unsupported("Cargo metadata path escapes the repository"))?;
    let normalized = relative
        .components()
        .map(|part| part.as_os_str().to_str())
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| unsupported("Cargo metadata path is not UTF-8"))?
        .join("/");
    if (!allow_root && normalized.is_empty())
        || normalized.contains('\0')
        || (!normalized.is_empty()
            && normalized
                .split('/')
                .any(|part| part.is_empty() || matches!(part, "." | "..")))
    {
        return Err(unsupported("Cargo metadata relative path is unsafe"));
    }
    Ok(normalized)
}

fn required_path<'a>(value: &'a Value, field: &str) -> Result<&'a Path, ClewError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(Path::new)
        .ok_or_else(|| unsupported("Cargo metadata path field is missing"))
}

fn required_string<'a>(value: &'a Value, field: &str) -> Result<&'a str, ClewError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| safe_component(value))
        .ok_or_else(|| unsupported("Cargo metadata identity field is invalid"))
}

fn require_regular_file(path: &Path, label: &str) -> Result<(), ClewError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| unsupported(label))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(unsupported(label));
    }
    Ok(())
}

fn safe_relative_manifest(path: &str) -> bool {
    path.ends_with("Cargo.toml")
        && !path.starts_with('/')
        && !path.contains('\0')
        && path
            .split('/')
            .all(|part| !part.is_empty() && !matches!(part, "." | ".." | ".git"))
}

fn safe_relative_input(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\0')
        && path
            .split('/')
            .all(|part| !part.is_empty() && !matches!(part, "." | ".." | ".git"))
}

fn canonical_digest(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn safe_component(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

fn invalid(message: &str) -> ClewError {
    ClewError::new(ErrorCode::InvalidInput, message)
}

fn unsupported(message: &str) -> ClewError {
    ClewError::new(ErrorCode::UnsupportedProjectConfiguration, message)
}

fn internal(error: impl std::fmt::Display) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::{RustCompilationSelector, extract_cargo_model};
    use std::fs;
    use std::path::Path;

    #[test]
    fn selector_is_exact_canonical_and_path_safe() {
        let value = "cargo:crates/clew/Cargo.toml#clew#lib#clew";
        let selector = RustCompilationSelector::parse(value).unwrap();
        assert_eq!(selector.canonical(), value);
        for invalid in [
            ":/main",
            "cargo:/Cargo.toml#clew#lib#clew",
            "cargo:../Cargo.toml#clew#lib#clew",
            "cargo:Cargo.toml#clew#lib",
            "cargo:Cargo.toml#clew#lib#bad/name",
        ] {
            assert!(
                RustCompilationSelector::parse(invalid).is_err(),
                "{invalid}"
            );
        }
    }

    #[test]
    fn mixed_repository_model_is_exact_and_contains_no_private_paths() {
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("fixtures/rust-mixed");
        let requested = [
            "cargo:Cargo.toml#rust-mixed-fixture#bin#rust-mixed-fixture".to_owned(),
            "cargo:Cargo.toml#rust-mixed-fixture#lib#rust_mixed_fixture".to_owned(),
        ];
        let model = extract_cargo_model(&repository, &requested).unwrap();
        assert_eq!(model.targets.len(), 2);
        assert_eq!(model.workspace_manifest, "Cargo.toml");
        assert_eq!(
            model
                .targets
                .iter()
                .map(|target| target.source_path.as_str())
                .collect::<Vec<_>>(),
            ["src/main.rs", "src/lib.rs"]
        );
        assert!(model.inputs.iter().any(|input| input.path == "Cargo.lock"));
        assert!(
            model
                .inputs
                .iter()
                .any(|input| input.path == "rust-toolchain.toml")
        );
        assert!(model.rustc_version.starts_with("rustc 1.92.0"));
        let encoded = serde_json::to_string(&model).unwrap();
        assert!(!encoded.contains(env!("CARGO_MANIFEST_DIR")));
        assert!(!encoded.contains("file://"));
        assert_eq!(
            extract_cargo_model(&repository, &requested)
                .unwrap()
                .model_digest,
            model.model_digest
        );
        let mut tampered = model.clone();
        tampered.targets[0].source_path = "/private/source.rs".into();
        assert!(tampered.verify().is_err());
    }

    #[cfg(unix)]
    #[test]
    fn cargo_target_symlink_is_refused_before_resolution() {
        use std::os::unix::fs::symlink;

        let repository = tempfile::tempdir().unwrap();
        fs::create_dir(repository.path().join("src")).unwrap();
        fs::write(
            repository.path().join("Cargo.toml"),
            b"[package]\nname='linked'\nversion='0.1.0'\nedition='2024'\n[workspace]\n",
        )
        .unwrap();
        fs::write(
            repository.path().join("Cargo.lock"),
            b"version = 4\n\n[[package]]\nname = \"linked\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        fs::write(
            repository.path().join("src/real.rs"),
            b"pub fn value() {}\n",
        )
        .unwrap();
        symlink("real.rs", repository.path().join("src/lib.rs")).unwrap();
        let error = extract_cargo_model(
            repository.path(),
            &["cargo:Cargo.toml#linked#lib#linked".into()],
        )
        .unwrap_err();
        assert_eq!(
            error.code,
            crate::error::ErrorCode::UnsupportedProjectConfiguration
        );
        assert!(
            !error
                .message
                .contains(repository.path().to_string_lossy().as_ref())
        );
    }

    #[test]
    fn version_command_runs_under_repository_toolchain_cwd() {
        let repository = tempfile::tempdir().unwrap();
        let observed =
            super::command_version(repository.path(), "sh", &["-c", "pwd"], "cwd query failed")
                .unwrap();
        assert_eq!(
            Path::new(&observed).canonicalize().unwrap(),
            repository.path().canonicalize().unwrap()
        );
    }
}
