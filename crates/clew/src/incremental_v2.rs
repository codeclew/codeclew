use crate::adapter_v2::CompilationDescriptor;
use crate::canonical;
use crate::cas::CasObject;
use crate::error::{ClewError, ErrorCode};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub const COMPILER_STORE_KEY_SCHEMA: &str = "codeclew-compiler-store-key/2.0";
pub const INCREMENTAL_RECEIPT_SCHEMA: &str = "codeclew-incremental-receipt/2.0";
pub const COMPLETENESS_VECTOR_SCHEMA: &str = "codeclew-completeness-vector/2.0";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompilerStoreKey {
    pub schema: String,
    pub key: String,
    pub adapter_id: String,
    pub adapter_digest: String,
    pub language_uri: String,
    pub toolchain: CasObject,
    pub canonical_options: CasObject,
    pub classpath: Vec<CasObject>,
    pub plugins: Vec<CasObject>,
}

impl CompilerStoreKey {
    pub fn create(
        adapter_id: impl Into<String>,
        adapter_digest: impl Into<String>,
        compilation: &CompilationDescriptor,
    ) -> Result<Self, ClewError> {
        compilation.validate()?;
        let mut value = Self {
            schema: COMPILER_STORE_KEY_SCHEMA.into(),
            key: String::new(),
            adapter_id: adapter_id.into(),
            adapter_digest: adapter_digest.into(),
            language_uri: compilation.language_uri.as_str().into(),
            toolchain: compilation.toolchain.clone(),
            canonical_options: compilation.canonical_options.clone(),
            classpath: compilation.classpath.clone(),
            plugins: compilation.plugins.clone(),
        };
        value.classpath.sort_by(cas_order);
        value.plugins.sort_by(cas_order);
        if !safe_id(&value.adapter_id) || !digest(&value.adapter_digest) {
            return Err(invalid("compiler store adapter authority is invalid"));
        }
        value.key = canonical::hash(&value).map_err(internal)?;
        Ok(value)
    }

    pub fn path_component(&self) -> Result<&str, ClewError> {
        self.key
            .strip_prefix("sha256:")
            .filter(|value| value.len() == 64)
            .ok_or_else(|| invalid("compiler store key is invalid"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FileReceipt {
    pub path: String,
    pub content_digest: String,
    pub exported_surface_digest: String,
    pub dependencies: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BoundaryReceipt {
    pub source_path: String,
    pub target_path: String,
    pub boundary_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IncrementalReceipt {
    pub schema: String,
    pub compiler_store_key: String,
    pub generation_id: String,
    pub files: Vec<FileReceipt>,
    pub boundaries: Vec<BoundaryReceipt>,
    pub completeness: CompletenessVector,
}

impl IncrementalReceipt {
    pub fn validate(&self) -> Result<(), ClewError> {
        if self.schema != INCREMENTAL_RECEIPT_SCHEMA
            || !digest(&self.compiler_store_key)
            || !digest(&self.generation_id)
            || !self
                .files
                .windows(2)
                .all(|pair| pair[0].path < pair[1].path)
            || !self.boundaries.windows(2).all(|pair| {
                (&pair[0].source_path, &pair[0].target_path)
                    < (&pair[1].source_path, &pair[1].target_path)
            })
        {
            return Err(invalid("incremental receipt identity is invalid"));
        }
        for file in &self.files {
            if !safe_path(&file.path)
                || !digest(&file.content_digest)
                || !digest(&file.exported_surface_digest)
                || !file.dependencies.windows(2).all(|pair| pair[0] < pair[1])
                || file.dependencies.iter().any(|path| !safe_path(path))
            {
                return Err(invalid("incremental file receipt is invalid"));
            }
        }
        for boundary in &self.boundaries {
            if !safe_path(&boundary.source_path)
                || !safe_path(&boundary.target_path)
                || !digest(&boundary.boundary_digest)
            {
                return Err(invalid("incremental boundary receipt is invalid"));
            }
        }
        self.completeness.validate()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Support {
    Supported,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "status",
    rename_all = "SCREAMING_SNAKE_CASE",
    deny_unknown_fields
)]
pub enum Coverage {
    Complete {
        scope_digest: String,
    },
    Partial {
        observed_scopes: Vec<String>,
        boundaries: Vec<String>,
    },
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "status",
    rename_all = "SCREAMING_SNAKE_CASE",
    deny_unknown_fields
)]
pub enum Certainty {
    Verified,
    Unsure { check_set: Vec<String> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VerificationObligation {
    pub code: String,
    pub subject: Vec<String>,
    pub publication_blocking: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompletenessVector {
    pub schema: String,
    pub support: Support,
    pub coverage: Coverage,
    pub certainty: Certainty,
    pub obligations: Vec<VerificationObligation>,
}

impl CompletenessVector {
    pub fn verified_complete(scope_digest: String) -> Result<Self, ClewError> {
        let value = Self {
            schema: COMPLETENESS_VECTOR_SCHEMA.into(),
            support: Support::Supported,
            coverage: Coverage::Complete { scope_digest },
            certainty: Certainty::Verified,
            obligations: Vec::new(),
        };
        value.validate()?;
        Ok(value)
    }

    pub fn meet(&self, other: &Self) -> Result<Self, ClewError> {
        self.validate()?;
        other.validate()?;
        let support = if self.support == Support::Supported && other.support == Support::Supported {
            Support::Supported
        } else {
            Support::Unsupported
        };
        let coverage = match (&self.coverage, &other.coverage) {
            (
                Coverage::Complete { scope_digest: left },
                Coverage::Complete {
                    scope_digest: right,
                },
            ) if left == right => Coverage::Complete {
                scope_digest: left.clone(),
            },
            (Coverage::Unknown, _) | (_, Coverage::Unknown) => Coverage::Unknown,
            (left, right) => {
                let mut observed = coverage_scopes(left);
                observed.extend(coverage_scopes(right));
                let mut boundaries = coverage_boundaries(left);
                boundaries.extend(coverage_boundaries(right));
                Coverage::Partial {
                    observed_scopes: observed.into_iter().collect(),
                    boundaries: boundaries.into_iter().collect(),
                }
            }
        };
        let certainty = match (&self.certainty, &other.certainty) {
            (Certainty::Verified, Certainty::Verified) => Certainty::Verified,
            (left, right) => {
                let mut checks = certainty_checks(left);
                checks.extend(certainty_checks(right));
                Certainty::Unsure {
                    check_set: checks.into_iter().collect(),
                }
            }
        };
        let mut obligations = self.obligations.clone();
        obligations.extend(other.obligations.clone());
        obligations.sort_by(|left, right| {
            (&left.code, &left.subject, left.publication_blocking).cmp(&(
                &right.code,
                &right.subject,
                right.publication_blocking,
            ))
        });
        obligations.dedup();
        let value = Self {
            schema: COMPLETENESS_VECTOR_SCHEMA.into(),
            support,
            coverage,
            certainty,
            obligations,
        };
        value.validate()?;
        Ok(value)
    }

    pub fn publishable(&self) -> bool {
        self.support == Support::Supported
            && matches!(self.coverage, Coverage::Complete { .. })
            && self.certainty == Certainty::Verified
            && self.obligations.is_empty()
    }

    pub fn validate(&self) -> Result<(), ClewError> {
        if self.schema != COMPLETENESS_VECTOR_SCHEMA {
            return Err(invalid("completeness vector schema is invalid"));
        }
        match &self.coverage {
            Coverage::Complete { scope_digest } if !digest(scope_digest) => {
                return Err(invalid("complete coverage scope is invalid"));
            }
            Coverage::Partial {
                observed_scopes,
                boundaries,
            } if observed_scopes.windows(2).any(|pair| pair[0] >= pair[1])
                || boundaries.windows(2).any(|pair| pair[0] >= pair[1]) =>
            {
                return Err(invalid("partial coverage sets are not canonical"));
            }
            _ => {}
        }
        if let Certainty::Unsure { check_set } = &self.certainty
            && (check_set.is_empty() || check_set.windows(2).any(|pair| pair[0] >= pair[1]))
        {
            return Err(invalid("UNSURE evidence has no canonical check set"));
        }
        if self.obligations.windows(2).any(|pair| {
            (
                &pair[0].code,
                &pair[0].subject,
                pair[0].publication_blocking,
            ) >= (
                &pair[1].code,
                &pair[1].subject,
                pair[1].publication_blocking,
            )
        }) || self.obligations.iter().any(|obligation| {
            obligation.code.is_empty()
                || obligation.code.len() > 128
                || obligation.subject.is_empty()
        }) {
            return Err(invalid("verification obligations are not canonical"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "SCREAMING_SNAKE_CASE", deny_unknown_fields)]
pub enum IncrementalPlan {
    UnchangedHit {
        parent_generation_id: String,
    },
    Delta {
        parent_generation_id: String,
        changed_files: Vec<String>,
        invalidated_files: Vec<String>,
    },
    Full {
        reason: FullAnalysisReason,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FullAnalysisReason {
    NoParent,
    ConfigurationChanged,
    UnknownInvalidation,
    IncompleteParent,
    InvalidReceipt,
}

pub fn plan_incremental(
    compiler_store: &CompilerStoreKey,
    parent: Option<&IncrementalReceipt>,
    current_files: &BTreeMap<String, String>,
    invalidation_is_authoritative: bool,
) -> Result<IncrementalPlan, ClewError> {
    let Some(parent) = parent else {
        return Ok(IncrementalPlan::Full {
            reason: FullAnalysisReason::NoParent,
        });
    };
    if parent.validate().is_err() {
        return Ok(IncrementalPlan::Full {
            reason: FullAnalysisReason::InvalidReceipt,
        });
    }
    if parent.compiler_store_key != compiler_store.key {
        return Ok(IncrementalPlan::Full {
            reason: FullAnalysisReason::ConfigurationChanged,
        });
    }
    if !parent.completeness.publishable() {
        return Ok(IncrementalPlan::Full {
            reason: FullAnalysisReason::IncompleteParent,
        });
    }
    if !invalidation_is_authoritative {
        return Ok(IncrementalPlan::Full {
            reason: FullAnalysisReason::UnknownInvalidation,
        });
    }
    if current_files
        .iter()
        .any(|(path, digest_value)| !safe_path(path) || !digest(digest_value))
    {
        return Err(invalid("current incremental inputs are invalid"));
    }
    let parent_files = parent
        .files
        .iter()
        .map(|receipt| (receipt.path.as_str(), receipt))
        .collect::<BTreeMap<_, _>>();
    let mut changed = BTreeSet::new();
    for (path, digest_value) in current_files {
        if parent_files
            .get(path.as_str())
            .is_none_or(|receipt| receipt.content_digest != *digest_value)
        {
            changed.insert(path.clone());
        }
    }
    for path in parent_files.keys() {
        if !current_files.contains_key(*path) {
            changed.insert((*path).to_owned());
        }
    }
    if changed.is_empty() {
        return Ok(IncrementalPlan::UnchangedHit {
            parent_generation_id: parent.generation_id.clone(),
        });
    }
    let mut reverse = BTreeMap::<String, BTreeSet<String>>::new();
    for file in &parent.files {
        for dependency in &file.dependencies {
            reverse
                .entry(dependency.clone())
                .or_default()
                .insert(file.path.clone());
        }
    }
    let mut invalidated = changed.clone();
    let mut queue = changed.iter().cloned().collect::<VecDeque<_>>();
    while let Some(path) = queue.pop_front() {
        for dependent in reverse.get(&path).into_iter().flatten() {
            if invalidated.insert(dependent.clone()) {
                queue.push_back(dependent.clone());
            }
        }
    }
    Ok(IncrementalPlan::Delta {
        parent_generation_id: parent.generation_id.clone(),
        changed_files: changed.into_iter().collect(),
        invalidated_files: invalidated.into_iter().collect(),
    })
}

fn coverage_scopes(value: &Coverage) -> BTreeSet<String> {
    match value {
        Coverage::Complete { scope_digest } => BTreeSet::from([scope_digest.clone()]),
        Coverage::Partial {
            observed_scopes, ..
        } => observed_scopes.iter().cloned().collect(),
        Coverage::Unknown => BTreeSet::new(),
    }
}

fn coverage_boundaries(value: &Coverage) -> BTreeSet<String> {
    match value {
        Coverage::Partial { boundaries, .. } => boundaries.iter().cloned().collect(),
        _ => BTreeSet::new(),
    }
}

fn certainty_checks(value: &Certainty) -> BTreeSet<String> {
    match value {
        Certainty::Verified => BTreeSet::new(),
        Certainty::Unsure { check_set } => check_set.iter().cloned().collect(),
    }
}

fn cas_order(left: &CasObject, right: &CasObject) -> std::cmp::Ordering {
    (&left.object_schema, &left.digest, left.size).cmp(&(
        &right.object_schema,
        &right.digest,
        right.size,
    ))
}

fn safe_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn safe_path(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 4096
        && !value.starts_with('/')
        && !value.contains('\0')
        && !value
            .split('/')
            .any(|component| component.is_empty() || component == "." || component == "..")
}

fn digest(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
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

    fn d(character: char) -> String {
        format!("sha256:{}", character.to_string().repeat(64))
    }

    fn complete() -> CompletenessVector {
        CompletenessVector::verified_complete(d('a')).unwrap()
    }

    fn receipt() -> IncrementalReceipt {
        IncrementalReceipt {
            schema: INCREMENTAL_RECEIPT_SCHEMA.into(),
            compiler_store_key: d('b'),
            generation_id: d('c'),
            files: vec![
                FileReceipt {
                    path: "A.kt".into(),
                    content_digest: d('1'),
                    exported_surface_digest: d('2'),
                    dependencies: vec![],
                },
                FileReceipt {
                    path: "B.kt".into(),
                    content_digest: d('3'),
                    exported_surface_digest: d('4'),
                    dependencies: vec!["A.kt".into()],
                },
                FileReceipt {
                    path: "C.kt".into(),
                    content_digest: d('5'),
                    exported_surface_digest: d('6'),
                    dependencies: vec!["B.kt".into()],
                },
            ],
            boundaries: vec![],
            completeness: complete(),
        }
    }

    fn key(value: &str) -> CompilerStoreKey {
        CompilerStoreKey {
            schema: COMPILER_STORE_KEY_SCHEMA.into(),
            key: value.into(),
            adapter_id: "kotlin-2.4".into(),
            adapter_digest: d('d'),
            language_uri: "language:kotlin".into(),
            toolchain: object('e'),
            canonical_options: object('f'),
            classpath: vec![],
            plugins: vec![],
        }
    }

    fn object(character: char) -> CasObject {
        CasObject {
            schema: crate::cas::CAS_OBJECT_SCHEMA.into(),
            object_schema: "test/object/1".into(),
            digest: d(character),
            size: 1,
        }
    }

    #[test]
    fn unchanged_and_reverse_dependency_delta_are_exact() {
        let receipt = receipt();
        let current = BTreeMap::from([
            ("A.kt".into(), d('1')),
            ("B.kt".into(), d('3')),
            ("C.kt".into(), d('5')),
        ]);
        assert!(matches!(
            plan_incremental(&key(&d('b')), Some(&receipt), &current, true).unwrap(),
            IncrementalPlan::UnchangedHit { .. }
        ));
        let mut changed = current;
        changed.insert("A.kt".into(), d('9'));
        assert_eq!(
            plan_incremental(&key(&d('b')), Some(&receipt), &changed, true).unwrap(),
            IncrementalPlan::Delta {
                parent_generation_id: d('c'),
                changed_files: vec!["A.kt".into()],
                invalidated_files: vec!["A.kt".into(), "B.kt".into(), "C.kt".into()],
            }
        );
    }

    #[test]
    fn config_unknown_and_unsure_never_reuse_complete_parent() {
        let mut receipt = receipt();
        let current = BTreeMap::from([
            ("A.kt".into(), d('1')),
            ("B.kt".into(), d('3')),
            ("C.kt".into(), d('5')),
        ]);
        assert!(matches!(
            plan_incremental(&key(&d('9')), Some(&receipt), &current, true).unwrap(),
            IncrementalPlan::Full {
                reason: FullAnalysisReason::ConfigurationChanged
            }
        ));
        assert!(matches!(
            plan_incremental(&key(&d('b')), Some(&receipt), &current, false).unwrap(),
            IncrementalPlan::Full {
                reason: FullAnalysisReason::UnknownInvalidation
            }
        ));
        receipt.completeness.certainty = Certainty::Unsure {
            check_set: vec!["run-test".into()],
        };
        assert!(matches!(
            plan_incremental(&key(&d('b')), Some(&receipt), &current, true).unwrap(),
            IncrementalPlan::Full {
                reason: FullAnalysisReason::IncompleteParent
            }
        ));
    }

    #[test]
    fn completeness_meet_is_commutative_idempotent_and_no_upgrade() {
        let complete = complete();
        let unsure = CompletenessVector {
            schema: COMPLETENESS_VECTOR_SCHEMA.into(),
            support: Support::Supported,
            coverage: Coverage::Partial {
                observed_scopes: vec![d('a')],
                boundaries: vec!["dynamic-call".into()],
            },
            certainty: Certainty::Unsure {
                check_set: vec!["integration-test".into()],
            },
            obligations: vec![VerificationObligation {
                code: "VERIFY_DYNAMIC_CALL".into(),
                subject: vec!["symbol:x".into()],
                publication_blocking: true,
            }],
        };
        let left = complete.meet(&unsure).unwrap();
        let right = unsure.meet(&complete).unwrap();
        assert_eq!(left, right);
        assert_eq!(unsure.meet(&unsure).unwrap(), unsure);
        assert!(!left.publishable());
        assert!(matches!(left.certainty, Certainty::Unsure { .. }));
    }
}
