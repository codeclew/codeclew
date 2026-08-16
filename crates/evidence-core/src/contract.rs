use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;

use crate::{canonical_json_bytes, sha256_digest};

pub const CONTRACT_VERSION: &str = "1.0.0";
pub const PROTOCOL_SCHEMA_PATH: &str = "schemas/evidence_core.proto";
pub const CORE_SPECIFICATION_PATH: &str = "contracts/core/evidence-core-v1.json";
pub const CONFORMANCE_SPECIFICATION_PATH: &str = "contracts/core/conformance-v1.json";
pub const CONTRACT_LOCK_PATH: &str = "contracts/core/core-contract.lock.json";

const DECISION_CORE_ROOT: &str = "crates/evidence-core";
const CONFORMANCE_TEST_ROOT: &str = "crates/evidence-core/tests";
const ADAPTER_CONTRACT_PATHS: &[&str] = &[
    "schemas/adapter_output.schema.json",
    "crates/evidence-adapters/src/lib.rs",
    "crates/evidence-adapters/src/core_bridge.rs",
    "crates/evidence-adapters/src/bin/evidence.rs",
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContractFile {
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContractDigests {
    pub protocol_schema_digest: String,
    pub core_specification_digest: String,
    pub conformance_specification_digest: String,
    pub decision_core_digest: String,
    pub conformance_corpus_digest: String,
    pub adapter_contract_digest: String,
    pub contract_digest: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrozenCoreContract {
    pub schema: String,
    pub contract_version: String,
    pub protocol_schema_path: String,
    pub core_specification_path: String,
    pub conformance_specification_path: String,
    pub decision_core_files: Vec<ContractFile>,
    pub conformance_corpus_files: Vec<ContractFile>,
    pub adapter_contract_files: Vec<ContractFile>,
    pub digests: ContractDigests,
}

#[derive(Debug, Error)]
pub enum ContractError {
    #[error("cannot read {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("cannot write {path}: {source}")]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("invalid JSON in {path}: {source}")]
    Json {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("{path} is not canonical JSON")]
    NonCanonicalJson { path: PathBuf },
    #[error("frozen core contract mismatch\nexpected: {expected:?}\nactual: {actual:?}")]
    Mismatch {
        expected: Box<FrozenCoreContract>,
        actual: Box<FrozenCoreContract>,
    },
    #[error("cannot canonicalize core contract: {0}")]
    Canonical(String),
}

impl FrozenCoreContract {
    pub fn compute(repository_root: impl AsRef<Path>) -> Result<Self, ContractError> {
        let root = repository_root.as_ref();
        let protocol = read(root.join(PROTOCOL_SCHEMA_PATH))?;
        let core = read_canonical_json(root.join(CORE_SPECIFICATION_PATH))?;
        let conformance = read_canonical_json(root.join(CONFORMANCE_SPECIFICATION_PATH))?;
        let protocol_schema_digest = sha256_digest(protocol);
        let core_specification_digest = sha256_digest(core);
        let conformance_specification_digest = sha256_digest(conformance);
        let decision_core_files = collect_files(root, DECISION_CORE_ROOT, decision_core_file)?;
        let conformance_corpus_files = collect_files(root, CONFORMANCE_TEST_ROOT, |_| true)?;
        let adapter_contract_files = collect_explicit_files(root, ADAPTER_CONTRACT_PATHS)?;
        let decision_core_digest = file_manifest_digest(&decision_core_files)?;
        let conformance_corpus_digest = sha256_digest(
            canonical_json_bytes(&json!({
                "conformanceSpecificationDigest": conformance_specification_digest,
                "files": conformance_corpus_files,
            }))
            .map_err(|error| ContractError::Canonical(error.to_string()))?,
        );
        let adapter_contract_digest = file_manifest_digest(&adapter_contract_files)?;
        let contract_material = json!({
            "adapterContractDigest": adapter_contract_digest,
            "contractVersion": CONTRACT_VERSION,
            "conformanceSpecificationDigest": conformance_specification_digest,
            "conformanceCorpusDigest": conformance_corpus_digest,
            "coreSpecificationDigest": core_specification_digest,
            "decisionCoreDigest": decision_core_digest,
            "protocolSchemaDigest": protocol_schema_digest,
        });
        let contract_digest = sha256_digest(
            canonical_json_bytes(&contract_material)
                .map_err(|error| ContractError::Canonical(error.to_string()))?,
        );
        Ok(Self {
            schema: "codeclew.core-contract-lock/1.0".to_owned(),
            contract_version: CONTRACT_VERSION.to_owned(),
            protocol_schema_path: PROTOCOL_SCHEMA_PATH.to_owned(),
            core_specification_path: CORE_SPECIFICATION_PATH.to_owned(),
            conformance_specification_path: CONFORMANCE_SPECIFICATION_PATH.to_owned(),
            decision_core_files,
            conformance_corpus_files,
            adapter_contract_files,
            digests: ContractDigests {
                protocol_schema_digest,
                core_specification_digest,
                conformance_specification_digest,
                decision_core_digest,
                conformance_corpus_digest,
                adapter_contract_digest,
                contract_digest,
            },
        })
    }

    pub fn load(repository_root: impl AsRef<Path>) -> Result<Self, ContractError> {
        let path = repository_root.as_ref().join(CONTRACT_LOCK_PATH);
        let bytes = read_canonical_json(&path)?;
        serde_json::from_slice(&bytes).map_err(|source| ContractError::Json { path, source })
    }

    pub fn verify(repository_root: impl AsRef<Path>) -> Result<Self, ContractError> {
        let root = repository_root.as_ref();
        let expected = Self::load(root)?;
        let actual = Self::compute(root)?;
        if expected != actual {
            return Err(ContractError::Mismatch {
                expected: Box::new(expected),
                actual: Box::new(actual),
            });
        }
        Ok(actual)
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, ContractError> {
        canonical_json_bytes(self).map_err(|error| ContractError::Canonical(error.to_string()))
    }

    pub fn write_lock(&self, repository_root: impl AsRef<Path>) -> Result<(), ContractError> {
        let path = repository_root.as_ref().join(CONTRACT_LOCK_PATH);
        let mut bytes = self.canonical_bytes()?;
        bytes.push(b'\n');
        fs::write(&path, bytes).map_err(|source| ContractError::Write { path, source })
    }
}

fn decision_core_file(relative: &Path) -> bool {
    relative == Path::new("Cargo.toml")
        || relative == Path::new("build.rs")
        || relative.starts_with("src")
        || relative.starts_with("tests")
}

fn collect_files(
    root: &Path,
    relative_root: &str,
    include: impl Fn(&Path) -> bool,
) -> Result<Vec<ContractFile>, ContractError> {
    let absolute_root = root.join(relative_root);
    let mut pending = vec![absolute_root.clone()];
    let mut files = Vec::new();
    while let Some(directory) = pending.pop() {
        let entries = fs::read_dir(&directory).map_err(|source| ContractError::Read {
            path: directory.clone(),
            source,
        })?;
        for entry in entries {
            let entry = entry.map_err(|source| ContractError::Read {
                path: directory.clone(),
                source,
            })?;
            let path = entry.path();
            let file_type = entry.file_type().map_err(|source| ContractError::Read {
                path: path.clone(),
                source,
            })?;
            if file_type.is_symlink() {
                return Err(ContractError::Canonical(format!(
                    "contract input is a symlink: {}",
                    path.display()
                )));
            }
            if file_type.is_dir() {
                if entry.file_name() != "target" {
                    pending.push(path);
                }
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            let relative_to_area = path
                .strip_prefix(&absolute_root)
                .map_err(|error| ContractError::Canonical(error.to_string()))?;
            if !include(relative_to_area) {
                continue;
            }
            let repository_relative = path
                .strip_prefix(root)
                .map_err(|error| ContractError::Canonical(error.to_string()))?
                .to_string_lossy()
                .replace('\\', "/");
            let bytes = read(&path)?;
            files.push(ContractFile {
                path: repository_relative,
                size: bytes.len() as u64,
                sha256: sha256_digest(bytes),
            });
        }
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(files)
}

fn collect_explicit_files(
    root: &Path,
    relative_paths: &[&str],
) -> Result<Vec<ContractFile>, ContractError> {
    let mut files = relative_paths
        .iter()
        .map(|relative| {
            let path = root.join(relative);
            let metadata = fs::symlink_metadata(&path).map_err(|source| ContractError::Read {
                path: path.clone(),
                source,
            })?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(ContractError::Canonical(format!(
                    "adapter contract input is not a regular file: {}",
                    path.display()
                )));
            }
            let bytes = read(&path)?;
            Ok(ContractFile {
                path: (*relative).to_owned(),
                size: bytes.len() as u64,
                sha256: sha256_digest(bytes),
            })
        })
        .collect::<Result<Vec<_>, ContractError>>()?;
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(files)
}

fn file_manifest_digest(files: &[ContractFile]) -> Result<String, ContractError> {
    Ok(sha256_digest(canonical_json_bytes(files).map_err(
        |error| ContractError::Canonical(error.to_string()),
    )?))
}

fn read(path: impl AsRef<Path>) -> Result<Vec<u8>, ContractError> {
    let path = path.as_ref();
    fs::read(path).map_err(|source| ContractError::Read {
        path: path.to_owned(),
        source,
    })
}

fn read_canonical_json(path: impl AsRef<Path>) -> Result<Vec<u8>, ContractError> {
    let path = path.as_ref();
    let bytes = read(path)?;
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|source| ContractError::Json {
            path: path.to_owned(),
            source,
        })?;
    let canonical = canonical_json_bytes(&value)
        .map_err(|error| ContractError::Canonical(error.to_string()))?;
    if bytes != canonical && bytes != [canonical.as_slice(), b"\n"].concat() {
        return Err(ContractError::NonCanonicalJson {
            path: path.to_owned(),
        });
    }
    Ok(canonical)
}
