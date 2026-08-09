//! Minimal executable semantics for cumulative evidence records.
//!
//! This module deliberately stores semantic atoms and source *references*, not
//! source text or executable transition programs.  It is therefore a lossy,
//! disposable projection that can always be rebuilt from an authoritative
//! repository snapshot.

use crate::model::Snapshot;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use thiserror::Error;

const MAX_IDENTITY_BYTES: usize = 512;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RecordId(pub String);

impl From<&str> for RecordId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RecordKind {
    ObservedFact,
    DerivedFact,
    DeclaredFact,
    Assumption,
    Hypothesis,
    Invariant,
    Obligation,
    Claim,
    Evidence,
    Unknown,
    Conflict,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompositeSnapshot {
    pub base_revision: String,
    pub project_model_hash: String,
    pub index_snapshot: String,
    pub compiler_version: String,
    pub classpath_hash: String,
}

impl CompositeSnapshot {
    /// Compatibility adapter for the snapshot already used by `ThreadIr`.
    pub fn from_index(snapshot: &Snapshot, classpath_hash: impl Into<String>) -> Self {
        Self {
            base_revision: snapshot.base_revision.clone(),
            project_model_hash: snapshot.project_model_hash.clone(),
            index_snapshot: snapshot.index_snapshot.clone(),
            compiler_version: snapshot.compiler_version.clone(),
            classpath_hash: classpath_hash.into(),
        }
    }

    fn validate(&self) -> Result<(), KernelError> {
        for (name, value) in [
            ("baseRevision", self.base_revision.as_str()),
            ("projectModelHash", self.project_model_hash.as_str()),
            ("indexSnapshot", self.index_snapshot.as_str()),
            ("compilerVersion", self.compiler_version.as_str()),
            ("classpathHash", self.classpath_hash.as_str()),
        ] {
            validate_identity(name, value)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidityInterval {
    pub valid_from: CompositeSnapshot,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invalidated_at: Option<CompositeSnapshot>,
}

impl ValidityInterval {
    pub fn current(snapshot: CompositeSnapshot) -> Self {
        Self {
            valid_from: snapshot,
            invalidated_at: None,
        }
    }

    /// K01 is deliberately conservative: a record is current only for the
    /// exact composite snapshot from which it was built.  Carry-forward and
    /// incremental recomputation belong to K03.
    pub fn is_current_at(&self, snapshot: &CompositeSnapshot) -> bool {
        self.invalidated_at.is_none() && &self.valid_from == snapshot
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Soundness {
    Tentative,
    Conservative,
    Sound,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SemanticRelation {
    Exists,
    Declares,
    HasType,
    Calls,
    Reads,
    Writes,
    DependsOn,
    Satisfies,
    Contradicts,
    Validates,
    KnowledgeStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SemanticValue {
    Entity(String),
    Type(String),
    Boolean(bool),
    Integer(i64),
    Hash(String),
    Absent,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticAtom {
    /// Stable semantic identity, never a source excerpt.
    pub subject: String,
    pub relation: SemanticRelation,
    pub value: SemanticValue,
}

impl SemanticAtom {
    fn validate(&self) -> Result<(), KernelError> {
        validate_identity("semantic subject", &self.subject)?;
        match &self.value {
            SemanticValue::Entity(value)
            | SemanticValue::Type(value)
            | SemanticValue::Hash(value) => validate_identity("semantic value", value),
            _ => Ok(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Provenance {
    Source {
        snapshot: CompositeSnapshot,
        path: String,
        content_hash: String,
        start_byte: u64,
        end_byte: u64,
    },
    IndexFact {
        snapshot: CompositeSnapshot,
        fact_key: String,
        fact_hash: String,
    },
    DerivedFrom {
        record_id: RecordId,
    },
    Declaration {
        actor_id: String,
        artifact_hash: String,
    },
    Validation {
        run_id: String,
        artifact_hash: String,
    },
}

impl Provenance {
    fn validate(&self) -> Result<(), KernelError> {
        match self {
            Provenance::Source {
                snapshot,
                path,
                content_hash,
                start_byte,
                end_byte,
            } => {
                snapshot.validate()?;
                validate_identity("source path", path)?;
                validate_identity("content hash", content_hash)?;
                if start_byte > end_byte {
                    return Err(KernelError::InvalidProvenance(
                        "source range starts after it ends".to_owned(),
                    ));
                }
                Ok(())
            }
            Provenance::IndexFact {
                snapshot,
                fact_key,
                fact_hash,
            } => {
                snapshot.validate()?;
                validate_identity("fact key", fact_key)?;
                validate_identity("fact hash", fact_hash)
            }
            Provenance::DerivedFrom { record_id } => {
                validate_identity("derived record id", &record_id.0)
            }
            Provenance::Declaration {
                actor_id,
                artifact_hash,
            } => {
                validate_identity("actor id", actor_id)?;
                validate_identity("artifact hash", artifact_hash)
            }
            Provenance::Validation {
                run_id,
                artifact_hash,
            } => {
                validate_identity("validation run id", run_id)?;
                validate_identity("artifact hash", artifact_hash)
            }
        }
    }

    fn source_matches(&self, snapshot: &CompositeSnapshot, path: &str, content_hash: &str) -> bool {
        matches!(
            self,
            Provenance::Source {
                snapshot: origin,
                path: origin_path,
                content_hash: origin_hash,
                ..
            } if origin == snapshot && origin_path == path && origin_hash == content_hash
        )
    }

    fn snapshot(&self) -> Option<&CompositeSnapshot> {
        match self {
            Provenance::Source { snapshot, .. } | Provenance::IndexFact { snapshot, .. } => {
                Some(snapshot)
            }
            Provenance::DerivedFrom { .. }
            | Provenance::Declaration { .. }
            | Provenance::Validation { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CoverageStatus {
    Complete,
    Partial,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum GapReason {
    UnsupportedFeature,
    ExternalBoundary,
    DynamicDispatch,
    Budget,
    MissingEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoverageGap {
    /// Stable identifier also used as the subject of its `Unknown` record.
    pub id: String,
    pub reason: GapReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoverageBoundary {
    pub scope: String,
    pub status: CoverageStatus,
    #[serde(default)]
    pub gaps: Vec<CoverageGap>,
}

impl CoverageBoundary {
    pub fn complete(scope: impl Into<String>) -> Self {
        Self {
            scope: scope.into(),
            status: CoverageStatus::Complete,
            gaps: Vec::new(),
        }
    }

    fn validate(&self) -> Result<(), KernelError> {
        validate_identity("coverage scope", &self.scope)?;
        match self.status {
            CoverageStatus::Complete if !self.gaps.is_empty() => Err(KernelError::InvalidCoverage(
                "complete coverage contains gaps".to_owned(),
            )),
            CoverageStatus::Partial | CoverageStatus::Unknown if self.gaps.is_empty() => {
                Err(KernelError::InvalidCoverage(
                    "non-complete coverage has no explicit gap".to_owned(),
                ))
            }
            _ => {
                let mut ids = BTreeSet::new();
                for gap in &self.gaps {
                    validate_identity("coverage gap id", &gap.id)?;
                    if !ids.insert(&gap.id) {
                        return Err(KernelError::InvalidCoverage(format!(
                            "duplicate coverage gap {}",
                            gap.id
                        )));
                    }
                }
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EvidencePurpose {
    Validation,
    Explanation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ResolutionState {
    NotApplicable,
    ObligationPending,
    ObligationDischarged { evidence_id: RecordId },
    ConflictUnresolved,
    ConflictResolved { evidence_id: RecordId },
    EvidenceOffered { purpose: EvidencePurpose },
    EvidenceAccepted { purpose: EvidencePurpose },
    UnknownOpen,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Invalidation {
    pub changed_record: RecordId,
    pub at_snapshot: CompositeSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Freshness {
    Current,
    Invalidated { cause: Invalidation },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticRecord {
    pub id: RecordId,
    pub kind: RecordKind,
    pub atom: SemanticAtom,
    pub provenance: Vec<Provenance>,
    pub validity: ValidityInterval,
    pub soundness: Soundness,
    pub resolution: ResolutionState,
    pub freshness: Freshness,
}

impl SemanticRecord {
    pub fn is_current_at(&self, snapshot: &CompositeSnapshot) -> bool {
        self.freshness == Freshness::Current && self.validity.is_current_at(snapshot)
    }

    fn validate_shape(&self, snapshot: &CompositeSnapshot) -> Result<(), KernelError> {
        validate_identity("record id", &self.id.0)?;
        self.atom.validate()?;
        if self.provenance.is_empty() {
            return Err(KernelError::MissingProvenance(self.id.clone()));
        }
        for provenance in &self.provenance {
            provenance.validate()?;
            if provenance
                .snapshot()
                .is_some_and(|origin| origin != &self.validity.valid_from)
            {
                return Err(KernelError::InvalidProvenance(format!(
                    "record {} has provenance from a different composite snapshot",
                    self.id.0
                )));
            }
        }
        if matches!(
            self.resolution,
            ResolutionState::EvidenceOffered {
                purpose: EvidencePurpose::Validation
            } | ResolutionState::EvidenceAccepted {
                purpose: EvidencePurpose::Validation
            }
        ) && !self
            .provenance
            .iter()
            .any(|origin| matches!(origin, Provenance::Validation { .. }))
        {
            return Err(KernelError::InvalidProvenance(format!(
                "validation evidence {} has no validation-run provenance",
                self.id.0
            )));
        }
        self.validity.valid_from.validate()?;
        if let Some(invalidated_at) = &self.validity.invalidated_at {
            invalidated_at.validate()?;
        }
        if self.freshness == Freshness::Current && !self.validity.is_current_at(snapshot) {
            return Err(KernelError::StaleRecord(self.id.clone()));
        }
        let valid_resolution = matches!(
            (&self.kind, &self.resolution),
            (RecordKind::Obligation, ResolutionState::ObligationPending)
                | (
                    RecordKind::Obligation,
                    ResolutionState::ObligationDischarged { .. }
                )
                | (RecordKind::Conflict, ResolutionState::ConflictUnresolved)
                | (
                    RecordKind::Conflict,
                    ResolutionState::ConflictResolved { .. }
                )
                | (
                    RecordKind::Evidence,
                    ResolutionState::EvidenceOffered { .. }
                )
                | (
                    RecordKind::Evidence,
                    ResolutionState::EvidenceAccepted { .. }
                )
                | (RecordKind::Unknown, ResolutionState::UnknownOpen)
                | (
                    RecordKind::ObservedFact
                        | RecordKind::DerivedFact
                        | RecordKind::DeclaredFact
                        | RecordKind::Assumption
                        | RecordKind::Hypothesis
                        | RecordKind::Invariant
                        | RecordKind::Claim,
                    ResolutionState::NotApplicable
                )
        );
        if !valid_resolution {
            return Err(KernelError::InvalidResolution(self.id.clone()));
        }
        if self.kind == RecordKind::Unknown
            && (self.atom.relation != SemanticRelation::KnowledgeStatus
                || self.atom.value != SemanticValue::Unknown)
        {
            return Err(KernelError::InvalidResolution(self.id.clone()));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DependencyReason {
    Derivation,
    Justification,
    Validation,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DependencyRelation {
    pub dependent: RecordId,
    pub prerequisite: RecordId,
    pub reason: DependencyReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticKernel {
    snapshot: CompositeSnapshot,
    coverage: CoverageBoundary,
    records: BTreeMap<RecordId, SemanticRecord>,
    dependencies: BTreeSet<DependencyRelation>,
}

impl SemanticKernel {
    pub fn new(
        snapshot: CompositeSnapshot,
        coverage: CoverageBoundary,
    ) -> Result<Self, KernelError> {
        snapshot.validate()?;
        coverage.validate()?;
        Ok(Self {
            snapshot,
            coverage,
            records: BTreeMap::new(),
            dependencies: BTreeSet::new(),
        })
    }

    pub fn snapshot(&self) -> &CompositeSnapshot {
        &self.snapshot
    }

    pub fn coverage(&self) -> &CoverageBoundary {
        &self.coverage
    }

    pub fn records(&self) -> impl Iterator<Item = &SemanticRecord> {
        self.records.values()
    }

    pub fn record(&self, id: &RecordId) -> Option<&SemanticRecord> {
        self.records.get(id)
    }

    pub fn dependency_relations(&self) -> impl Iterator<Item = &DependencyRelation> {
        self.dependencies.iter()
    }

    pub fn insert(&mut self, record: SemanticRecord) -> Result<(), KernelError> {
        record.validate_shape(&self.snapshot)?;
        if self.records.contains_key(&record.id) {
            return Err(KernelError::DuplicateRecord(record.id));
        }
        let derived_from: Vec<_> = record
            .provenance
            .iter()
            .filter_map(|origin| match origin {
                Provenance::DerivedFrom { record_id } => Some(record_id.clone()),
                _ => None,
            })
            .collect();
        for prerequisite in &derived_from {
            if !self.records.contains_key(prerequisite) {
                return Err(KernelError::DanglingDerivedProvenance {
                    record: record.id.clone(),
                    prerequisite: prerequisite.clone(),
                });
            }
        }
        let id = record.id.clone();
        self.records.insert(id.clone(), record);
        for prerequisite in derived_from {
            if let Err(error) =
                self.add_dependency(&id, &prerequisite, DependencyReason::Derivation)
            {
                self.records.remove(&id);
                self.dependencies
                    .retain(|relation| relation.dependent != id);
                return Err(error);
            }
        }
        Ok(())
    }

    pub fn add_dependency(
        &mut self,
        dependent: &RecordId,
        prerequisite: &RecordId,
        reason: DependencyReason,
    ) -> Result<(), KernelError> {
        let dependent_record = self
            .records
            .get(dependent)
            .ok_or_else(|| KernelError::UnknownRecord(dependent.clone()))?;
        let prerequisite_record = self
            .records
            .get(prerequisite)
            .ok_or_else(|| KernelError::UnknownRecord(prerequisite.clone()))?;
        if dependent_record.soundness > prerequisite_record.soundness {
            return Err(KernelError::SoundnessEscalation {
                dependent: dependent.clone(),
                prerequisite: prerequisite.clone(),
            });
        }
        if !prerequisite_record.is_current_at(&self.snapshot) {
            return Err(KernelError::StaleRecord(prerequisite.clone()));
        }
        let relation = DependencyRelation {
            dependent: dependent.clone(),
            prerequisite: prerequisite.clone(),
            reason,
        };
        self.dependencies.insert(relation.clone());
        if let Err(error) = self.ensure_acyclic() {
            self.dependencies.remove(&relation);
            return Err(error);
        }
        Ok(())
    }

    pub fn invalidate(
        &mut self,
        changed: &RecordId,
        at_snapshot: CompositeSnapshot,
    ) -> Result<BTreeSet<RecordId>, KernelError> {
        if !self.records.contains_key(changed) {
            return Err(KernelError::UnknownRecord(changed.clone()));
        }
        at_snapshot.validate()?;
        let mut invalidated = BTreeSet::new();
        let mut queue = VecDeque::from([changed.clone()]);
        while let Some(current) = queue.pop_front() {
            if !invalidated.insert(current.clone()) {
                continue;
            }
            for relation in self
                .dependencies
                .iter()
                .filter(|relation| relation.prerequisite == current)
            {
                queue.push_back(relation.dependent.clone());
            }
        }
        for id in &invalidated {
            let record = self
                .records
                .get_mut(id)
                .expect("invalidation closure only contains known records");
            record.validity.invalidated_at = Some(at_snapshot.clone());
            record.freshness = Freshness::Invalidated {
                cause: Invalidation {
                    changed_record: changed.clone(),
                    at_snapshot: at_snapshot.clone(),
                },
            };
        }
        Ok(invalidated)
    }

    /// Handles disappearance of an authoritative source without retaining its
    /// bytes.  Every record with matching source provenance and every transitive
    /// dependent is conservatively invalidated.
    pub fn remove_source(
        &mut self,
        source_snapshot: &CompositeSnapshot,
        path: &str,
        content_hash: &str,
        at_snapshot: CompositeSnapshot,
    ) -> Result<BTreeSet<RecordId>, KernelError> {
        let roots: Vec<RecordId> = self
            .records
            .values()
            .filter(|record| {
                record
                    .provenance
                    .iter()
                    .any(|origin| origin.source_matches(source_snapshot, path, content_hash))
            })
            .map(|record| record.id.clone())
            .collect();
        let mut invalidated = BTreeSet::new();
        for root in roots {
            invalidated.extend(self.invalidate(&root, at_snapshot.clone())?);
        }
        Ok(invalidated)
    }

    pub fn accept_evidence(&mut self, id: &RecordId) -> Result<(), KernelError> {
        let record = self.current_record_mut(id)?;
        let purpose = match &record.resolution {
            ResolutionState::EvidenceOffered { purpose } => purpose.clone(),
            _ => return Err(KernelError::InvalidTransition(id.clone())),
        };
        record.resolution = ResolutionState::EvidenceAccepted { purpose };
        Ok(())
    }

    pub fn discharge_obligation(
        &mut self,
        obligation: &RecordId,
        evidence: &RecordId,
    ) -> Result<(), KernelError> {
        self.require_accepted_evidence(evidence)?;
        if self.current_record_mut(obligation)?.resolution != ResolutionState::ObligationPending {
            return Err(KernelError::InvalidTransition(obligation.clone()));
        }
        self.add_dependency(obligation, evidence, DependencyReason::Validation)?;
        self.records
            .get_mut(obligation)
            .expect("checked obligation exists")
            .resolution = ResolutionState::ObligationDischarged {
            evidence_id: evidence.clone(),
        };
        Ok(())
    }

    pub fn resolve_conflict(
        &mut self,
        conflict: &RecordId,
        evidence: &RecordId,
    ) -> Result<(), KernelError> {
        self.require_accepted_evidence(evidence)?;
        if self.current_record_mut(conflict)?.resolution != ResolutionState::ConflictUnresolved {
            return Err(KernelError::InvalidTransition(conflict.clone()));
        }
        self.add_dependency(conflict, evidence, DependencyReason::Validation)?;
        self.records
            .get_mut(conflict)
            .expect("checked conflict exists")
            .resolution = ResolutionState::ConflictResolved {
            evidence_id: evidence.clone(),
        };
        Ok(())
    }

    /// Remove only if the resulting model still conforms.  In particular, an
    /// explicit `Unknown` that represents a coverage gap cannot be erased.
    pub fn remove_record(&mut self, id: &RecordId) -> Result<SemanticRecord, KernelError> {
        let existing = self
            .records
            .get(id)
            .ok_or_else(|| KernelError::UnknownRecord(id.clone()))?;
        if matches!(
            existing.resolution,
            ResolutionState::ObligationPending | ResolutionState::ConflictUnresolved
        ) {
            return Err(KernelError::UnresolvedRecordRemoval(id.clone()));
        }
        let dependents: Vec<_> = self
            .dependencies
            .iter()
            .filter(|relation| &relation.prerequisite == id)
            .map(|relation| relation.dependent.clone())
            .collect();
        if !dependents.is_empty() {
            return Err(KernelError::RecordHasDependents {
                prerequisite: id.clone(),
                dependents,
            });
        }
        let record = self.records.remove(id).expect("checked record exists");
        let removed_relations: Vec<_> = self
            .dependencies
            .iter()
            .filter(|relation| &relation.dependent == id || &relation.prerequisite == id)
            .cloned()
            .collect();
        self.dependencies
            .retain(|relation| &relation.dependent != id && &relation.prerequisite != id);
        if let Err(error) = self.validate() {
            self.records.insert(id.clone(), record.clone());
            self.dependencies.extend(removed_relations);
            return Err(error);
        }
        Ok(record)
    }

    pub fn validate(&self) -> Result<(), KernelError> {
        self.snapshot.validate()?;
        self.coverage.validate()?;
        for record in self.records.values() {
            record.validate_shape(&self.snapshot)?;
        }
        for relation in &self.dependencies {
            let dependent = self
                .records
                .get(&relation.dependent)
                .ok_or_else(|| KernelError::UnknownRecord(relation.dependent.clone()))?;
            let prerequisite = self
                .records
                .get(&relation.prerequisite)
                .ok_or_else(|| KernelError::UnknownRecord(relation.prerequisite.clone()))?;
            if dependent.soundness > prerequisite.soundness {
                return Err(KernelError::SoundnessEscalation {
                    dependent: relation.dependent.clone(),
                    prerequisite: relation.prerequisite.clone(),
                });
            }
        }
        self.ensure_acyclic()?;
        self.ensure_invalidation_closure()?;
        self.ensure_derived_provenance_linked()?;
        self.ensure_unknowns_preserved()?;
        self.ensure_resolution_evidence()?;
        Ok(())
    }

    pub fn check_commit(&self, current: &CompositeSnapshot) -> Result<(), KernelError> {
        if current != &self.snapshot {
            return Err(KernelError::CurrentSnapshotRequired {
                expected: Box::new(self.snapshot.clone()),
                actual: Box::new(current.clone()),
            });
        }
        self.validate()?;
        let stale: Vec<_> = self
            .records
            .values()
            .filter(|record| !record.is_current_at(current))
            .map(|record| record.id.clone())
            .collect();
        if !stale.is_empty() {
            return Err(KernelError::StaleRecords(stale));
        }
        let obligations: Vec<_> = self
            .records
            .values()
            .filter(|record| record.resolution == ResolutionState::ObligationPending)
            .map(|record| record.id.clone())
            .collect();
        if !obligations.is_empty() {
            return Err(KernelError::UndischargedObligations(obligations));
        }
        let conflicts: Vec<_> = self
            .records
            .values()
            .filter(|record| record.resolution == ResolutionState::ConflictUnresolved)
            .map(|record| record.id.clone())
            .collect();
        if !conflicts.is_empty() {
            return Err(KernelError::UnresolvedConflicts(conflicts));
        }
        let accepted_validation = self.records.values().any(|record| {
            record.is_current_at(current)
                && record.resolution
                    == ResolutionState::EvidenceAccepted {
                        purpose: EvidencePurpose::Validation,
                    }
        });
        if !accepted_validation {
            return Err(KernelError::AcceptedValidationEvidenceRequired);
        }
        Ok(())
    }

    pub fn anti_duplication_inventory(&self) -> AntiDuplicationInventory {
        AntiDuplicationInventory {
            semantic_records: self.records.len(),
            provenance_links: self
                .records
                .values()
                .map(|record| record.provenance.len())
                .sum(),
            source_body_bytes: 0,
            transition_program_bytes: 0,
        }
    }

    fn current_record_mut(&mut self, id: &RecordId) -> Result<&mut SemanticRecord, KernelError> {
        let snapshot = self.snapshot.clone();
        let record = self
            .records
            .get_mut(id)
            .ok_or_else(|| KernelError::UnknownRecord(id.clone()))?;
        if !record.is_current_at(&snapshot) {
            return Err(KernelError::StaleRecord(id.clone()));
        }
        Ok(record)
    }

    fn require_accepted_evidence(&self, id: &RecordId) -> Result<(), KernelError> {
        let evidence = self
            .records
            .get(id)
            .ok_or_else(|| KernelError::UnknownRecord(id.clone()))?;
        if !evidence.is_current_at(&self.snapshot)
            || !matches!(
                evidence.resolution,
                ResolutionState::EvidenceAccepted { .. }
            )
        {
            return Err(KernelError::EvidenceNotAccepted(id.clone()));
        }
        Ok(())
    }

    fn ensure_acyclic(&self) -> Result<(), KernelError> {
        fn visit(
            id: &RecordId,
            dependencies: &BTreeSet<DependencyRelation>,
            temporary: &mut BTreeSet<RecordId>,
            permanent: &mut BTreeSet<RecordId>,
        ) -> Result<(), KernelError> {
            if permanent.contains(id) {
                return Ok(());
            }
            if !temporary.insert(id.clone()) {
                return Err(KernelError::CyclicDerivation(id.clone()));
            }
            for relation in dependencies
                .iter()
                .filter(|relation| &relation.dependent == id)
            {
                visit(&relation.prerequisite, dependencies, temporary, permanent)?;
            }
            temporary.remove(id);
            permanent.insert(id.clone());
            Ok(())
        }

        let mut temporary = BTreeSet::new();
        let mut permanent = BTreeSet::new();
        for id in self.records.keys() {
            visit(id, &self.dependencies, &mut temporary, &mut permanent)?;
        }
        Ok(())
    }

    fn ensure_invalidation_closure(&self) -> Result<(), KernelError> {
        for relation in &self.dependencies {
            let prerequisite = &self.records[&relation.prerequisite];
            let dependent = &self.records[&relation.dependent];
            if !prerequisite.is_current_at(&self.snapshot)
                && dependent.is_current_at(&self.snapshot)
            {
                return Err(KernelError::MissingDependentInvalidation {
                    prerequisite: relation.prerequisite.clone(),
                    dependent: relation.dependent.clone(),
                });
            }
        }
        Ok(())
    }

    fn ensure_derived_provenance_linked(&self) -> Result<(), KernelError> {
        for record in self.records.values() {
            for prerequisite in record.provenance.iter().filter_map(|origin| match origin {
                Provenance::DerivedFrom { record_id } => Some(record_id),
                _ => None,
            }) {
                if !self.records.contains_key(prerequisite) {
                    return Err(KernelError::DanglingDerivedProvenance {
                        record: record.id.clone(),
                        prerequisite: prerequisite.clone(),
                    });
                }
                let linked = self.dependencies.iter().any(|relation| {
                    relation.dependent == record.id
                        && relation.prerequisite == *prerequisite
                        && relation.reason == DependencyReason::Derivation
                });
                if !linked {
                    return Err(KernelError::UnlinkedDerivedProvenance {
                        record: record.id.clone(),
                        prerequisite: prerequisite.clone(),
                    });
                }
            }
        }
        Ok(())
    }

    fn ensure_unknowns_preserved(&self) -> Result<(), KernelError> {
        let represented: BTreeSet<&str> = self
            .records
            .values()
            .filter(|record| {
                record.kind == RecordKind::Unknown && record.is_current_at(&self.snapshot)
            })
            .map(|record| record.atom.subject.as_str())
            .collect();
        let missing: Vec<_> = self
            .coverage
            .gaps
            .iter()
            .filter(|gap| !represented.contains(gap.id.as_str()))
            .map(|gap| gap.id.clone())
            .collect();
        if missing.is_empty() {
            Ok(())
        } else {
            Err(KernelError::ErasedUnknown(missing))
        }
    }

    fn ensure_resolution_evidence(&self) -> Result<(), KernelError> {
        for record in self.records.values() {
            if !record.is_current_at(&self.snapshot) {
                continue;
            }
            let evidence_id = match &record.resolution {
                ResolutionState::ObligationDischarged { evidence_id }
                | ResolutionState::ConflictResolved { evidence_id } => evidence_id,
                _ => continue,
            };
            let evidence = self.records.get(evidence_id).ok_or_else(|| {
                KernelError::ResolutionEvidenceMissing {
                    record: record.id.clone(),
                    evidence: evidence_id.clone(),
                }
            })?;
            if !evidence.is_current_at(&self.snapshot)
                || !matches!(
                    evidence.resolution,
                    ResolutionState::EvidenceAccepted { .. }
                )
            {
                return Err(KernelError::EvidenceNotAccepted(evidence_id.clone()));
            }
            let linked = self.dependencies.iter().any(|relation| {
                relation.dependent == record.id
                    && relation.prerequisite == *evidence_id
                    && relation.reason == DependencyReason::Validation
            });
            if !linked {
                return Err(KernelError::ResolutionEvidenceUnlinked {
                    record: record.id.clone(),
                    evidence: evidence_id.clone(),
                });
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AntiDuplicationInventory {
    pub semantic_records: usize,
    pub provenance_links: usize,
    pub source_body_bytes: usize,
    pub transition_program_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum KernelError {
    #[error("invalid bounded identity for {field}: {reason}")]
    InvalidIdentity { field: String, reason: String },
    #[error("invalid provenance: {0}")]
    InvalidProvenance(String),
    #[error("record {0:?} has no provenance")]
    MissingProvenance(RecordId),
    #[error("duplicate semantic record {0:?}")]
    DuplicateRecord(RecordId),
    #[error("record {record:?} derives from missing prerequisite {prerequisite:?}")]
    DanglingDerivedProvenance {
        record: RecordId,
        prerequisite: RecordId,
    },
    #[error("record {record:?} lacks a derivation edge to provenance {prerequisite:?}")]
    UnlinkedDerivedProvenance {
        record: RecordId,
        prerequisite: RecordId,
    },
    #[error("unknown semantic record {0:?}")]
    UnknownRecord(RecordId),
    #[error("cannot remove unresolved obligation or conflict {0:?}")]
    UnresolvedRecordRemoval(RecordId),
    #[error("cannot remove prerequisite {prerequisite:?}; live dependents: {dependents:?}")]
    RecordHasDependents {
        prerequisite: RecordId,
        dependents: Vec<RecordId>,
    },
    #[error("record {0:?} is stale")]
    StaleRecord(RecordId),
    #[error("records are stale: {0:?}")]
    StaleRecords(Vec<RecordId>),
    #[error("record {0:?} has a resolution incompatible with its kind")]
    InvalidResolution(RecordId),
    #[error("invalid transition for record {0:?}")]
    InvalidTransition(RecordId),
    #[error("evidence {0:?} has not been accepted")]
    EvidenceNotAccepted(RecordId),
    #[error("record {record:?} references missing resolution evidence {evidence:?}")]
    ResolutionEvidenceMissing {
        record: RecordId,
        evidence: RecordId,
    },
    #[error("record {record:?} is not dependency-linked to resolution evidence {evidence:?}")]
    ResolutionEvidenceUnlinked {
        record: RecordId,
        evidence: RecordId,
    },
    #[error("invalid coverage boundary: {0}")]
    InvalidCoverage(String),
    #[error("explicit Unknown was erased for gaps {0:?}")]
    ErasedUnknown(Vec<String>),
    #[error("cyclic derivation through {0:?}")]
    CyclicDerivation(RecordId),
    #[error("soundness escalation from {prerequisite:?} to {dependent:?}")]
    SoundnessEscalation {
        dependent: RecordId,
        prerequisite: RecordId,
    },
    #[error("dependent {dependent:?} remained current after {prerequisite:?} became stale")]
    MissingDependentInvalidation {
        prerequisite: RecordId,
        dependent: RecordId,
    },
    #[error("commit snapshot is not current")]
    CurrentSnapshotRequired {
        expected: Box<CompositeSnapshot>,
        actual: Box<CompositeSnapshot>,
    },
    #[error("undischarged obligations: {0:?}")]
    UndischargedObligations(Vec<RecordId>),
    #[error("unresolved conflicts: {0:?}")]
    UnresolvedConflicts(Vec<RecordId>),
    #[error("commit needs accepted validation evidence")]
    AcceptedValidationEvidenceRequired,
}

fn validate_identity(field: &str, value: &str) -> Result<(), KernelError> {
    let reason = if value.is_empty() {
        Some("empty")
    } else if value.len() > MAX_IDENTITY_BYTES {
        Some("too long")
    } else if value.chars().any(char::is_control) {
        Some("contains control characters")
    } else {
        None
    };
    match reason {
        Some(reason) => Err(KernelError::InvalidIdentity {
            field: field.to_owned(),
            reason: reason.to_owned(),
        }),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(revision: &str) -> CompositeSnapshot {
        CompositeSnapshot {
            base_revision: revision.to_owned(),
            project_model_hash: format!("model-{revision}"),
            index_snapshot: format!("index-{revision}"),
            compiler_version: "2.4.10".to_owned(),
            classpath_hash: format!("classpath-{revision}"),
        }
    }

    fn record(id: &str, kind: RecordKind, snapshot: &CompositeSnapshot) -> SemanticRecord {
        let resolution = match kind {
            RecordKind::Obligation => ResolutionState::ObligationPending,
            RecordKind::Conflict => ResolutionState::ConflictUnresolved,
            RecordKind::Evidence => ResolutionState::EvidenceOffered {
                purpose: EvidencePurpose::Validation,
            },
            RecordKind::Unknown => ResolutionState::UnknownOpen,
            _ => ResolutionState::NotApplicable,
        };
        let atom = if kind == RecordKind::Unknown {
            SemanticAtom {
                subject: id.to_owned(),
                relation: SemanticRelation::KnowledgeStatus,
                value: SemanticValue::Unknown,
            }
        } else {
            SemanticAtom {
                subject: format!("entity-{id}"),
                relation: SemanticRelation::Exists,
                value: SemanticValue::Boolean(true),
            }
        };
        let provenance = if kind == RecordKind::Evidence {
            vec![Provenance::Validation {
                run_id: format!("run-{id}"),
                artifact_hash: format!("hash-{id}"),
            }]
        } else {
            vec![Provenance::IndexFact {
                snapshot: snapshot.clone(),
                fact_key: format!("fact-{id}"),
                fact_hash: format!("hash-{id}"),
            }]
        };
        SemanticRecord {
            id: id.into(),
            kind,
            atom,
            provenance,
            validity: ValidityInterval::current(snapshot.clone()),
            soundness: Soundness::Conservative,
            resolution,
            freshness: Freshness::Current,
        }
    }

    fn complete_kernel(snapshot: &CompositeSnapshot) -> SemanticKernel {
        SemanticKernel::new(snapshot.clone(), CoverageBoundary::complete("repository")).unwrap()
    }

    #[test]
    fn rejects_missing_provenance() {
        let current = snapshot("a");
        let mut kernel = complete_kernel(&current);
        let mut candidate = record("observed", RecordKind::ObservedFact, &current);
        candidate.provenance.clear();
        assert_eq!(
            kernel.insert(candidate),
            Err(KernelError::MissingProvenance("observed".into()))
        );
    }

    #[test]
    fn rejects_provenance_from_a_different_composite_snapshot() {
        let current = snapshot("a");
        let mut kernel = complete_kernel(&current);
        let mut candidate = record("observed", RecordKind::ObservedFact, &current);
        candidate.provenance = vec![Provenance::IndexFact {
            snapshot: snapshot("b"),
            fact_key: "fact-observed".to_owned(),
            fact_hash: "hash-observed".to_owned(),
        }];
        assert!(matches!(
            kernel.insert(candidate),
            Err(KernelError::InvalidProvenance(_))
        ));
    }

    #[test]
    fn derived_provenance_requires_and_creates_executable_lineage() {
        let current = snapshot("a");
        let mut kernel = complete_kernel(&current);
        let mut dangling = record("derived", RecordKind::DerivedFact, &current);
        dangling.provenance = vec![Provenance::DerivedFrom {
            record_id: "missing".into(),
        }];
        assert_eq!(
            kernel.insert(dangling.clone()),
            Err(KernelError::DanglingDerivedProvenance {
                record: "derived".into(),
                prerequisite: "missing".into(),
            })
        );

        kernel
            .insert(record("base", RecordKind::ObservedFact, &current))
            .unwrap();
        dangling.provenance = vec![Provenance::DerivedFrom {
            record_id: "base".into(),
        }];
        kernel.insert(dangling).unwrap();
        assert!(kernel.dependency_relations().any(|relation| {
            relation.dependent == "derived".into()
                && relation.prerequisite == "base".into()
                && relation.reason == DependencyReason::Derivation
        }));

        let invalidated = kernel.invalidate(&"base".into(), snapshot("b")).unwrap();
        assert_eq!(
            invalidated,
            BTreeSet::from(["base".into(), "derived".into()])
        );
        assert!(kernel.validate().is_ok());
    }

    #[test]
    fn validation_rejects_a_missing_derived_provenance_edge() {
        let current = snapshot("a");
        let mut kernel = complete_kernel(&current);
        kernel
            .insert(record("base", RecordKind::ObservedFact, &current))
            .unwrap();
        let mut derived = record("derived", RecordKind::DerivedFact, &current);
        derived.provenance = vec![Provenance::DerivedFrom {
            record_id: "base".into(),
        }];
        kernel.insert(derived).unwrap();
        kernel.dependencies.clear();

        assert_eq!(
            kernel.validate(),
            Err(KernelError::UnlinkedDerivedProvenance {
                record: "derived".into(),
                prerequisite: "base".into(),
            })
        );
    }

    #[test]
    fn deserialized_dependency_cannot_escalate_soundness() {
        let current = snapshot("a");
        let mut kernel = complete_kernel(&current);
        let mut base = record("base", RecordKind::ObservedFact, &current);
        base.soundness = Soundness::Sound;
        kernel.insert(base).unwrap();
        let mut derived = record("derived", RecordKind::DerivedFact, &current);
        derived.soundness = Soundness::Sound;
        derived.provenance = vec![Provenance::DerivedFrom {
            record_id: "base".into(),
        }];
        kernel.insert(derived).unwrap();

        let mut encoded = serde_json::to_value(&kernel).unwrap();
        encoded["records"]["base"]["soundness"] = serde_json::json!("TENTATIVE");
        let mutated: SemanticKernel = serde_json::from_value(encoded).unwrap();
        assert_eq!(
            mutated.validate(),
            Err(KernelError::SoundnessEscalation {
                dependent: "derived".into(),
                prerequisite: "base".into(),
            })
        );
    }

    #[test]
    fn rejects_validation_evidence_without_a_validation_run() {
        let current = snapshot("a");
        let mut kernel = complete_kernel(&current);
        let mut candidate = record("validation", RecordKind::Evidence, &current);
        candidate.provenance = vec![Provenance::IndexFact {
            snapshot: current.clone(),
            fact_key: "not-a-run".to_owned(),
            fact_hash: "hash".to_owned(),
        }];
        assert!(matches!(
            kernel.insert(candidate),
            Err(KernelError::InvalidProvenance(_))
        ));
    }

    #[test]
    fn rejects_stale_commit_snapshot() {
        let current = snapshot("a");
        let kernel = complete_kernel(&current);
        assert!(matches!(
            kernel.check_commit(&snapshot("b")),
            Err(KernelError::CurrentSnapshotRequired { .. })
        ));
    }

    #[test]
    fn invalidation_is_transitive_for_arbitrarily_long_dependency_chains() {
        let current = snapshot("a");
        for chain_length in 1..24 {
            let mut kernel = complete_kernel(&current);
            for index in 0..chain_length {
                let kind = if index == 0 {
                    RecordKind::ObservedFact
                } else {
                    RecordKind::DerivedFact
                };
                kernel
                    .insert(record(&format!("r{index}"), kind, &current))
                    .unwrap();
                if index > 0 {
                    kernel
                        .add_dependency(
                            &RecordId(format!("r{index}")),
                            &RecordId(format!("r{}", index - 1)),
                            DependencyReason::Derivation,
                        )
                        .unwrap();
                }
            }
            let invalidated = kernel
                .invalidate(&RecordId("r0".to_owned()), snapshot("b"))
                .unwrap();
            assert_eq!(invalidated.len(), chain_length);
            assert!(kernel.validate().is_ok());
        }
    }

    #[test]
    fn detects_missing_dependent_invalidation() {
        let current = snapshot("a");
        let mut kernel = complete_kernel(&current);
        kernel
            .insert(record("base", RecordKind::ObservedFact, &current))
            .unwrap();
        kernel
            .insert(record("derived", RecordKind::DerivedFact, &current))
            .unwrap();
        kernel
            .add_dependency(
                &"derived".into(),
                &"base".into(),
                DependencyReason::Derivation,
            )
            .unwrap();
        let base = kernel.records.get_mut(&RecordId::from("base")).unwrap();
        base.freshness = Freshness::Invalidated {
            cause: Invalidation {
                changed_record: "base".into(),
                at_snapshot: snapshot("b"),
            },
        };
        base.validity.invalidated_at = Some(snapshot("b"));
        assert_eq!(
            kernel.validate(),
            Err(KernelError::MissingDependentInvalidation {
                prerequisite: "base".into(),
                dependent: "derived".into(),
            })
        );
    }

    #[test]
    fn live_prerequisite_cannot_be_removed_from_under_a_derived_fact() {
        let current = snapshot("a");
        let mut kernel = complete_kernel(&current);
        kernel
            .insert(record("base", RecordKind::ObservedFact, &current))
            .unwrap();
        kernel
            .insert(record("derived", RecordKind::DerivedFact, &current))
            .unwrap();
        kernel
            .add_dependency(
                &"derived".into(),
                &"base".into(),
                DependencyReason::Derivation,
            )
            .unwrap();

        assert_eq!(
            kernel.remove_record(&"base".into()),
            Err(KernelError::RecordHasDependents {
                prerequisite: "base".into(),
                dependents: vec!["derived".into()],
            })
        );
        assert!(kernel.record(&"base".into()).is_some());
    }

    #[test]
    fn commit_rejects_undischarged_obligation() {
        let current = snapshot("a");
        let mut kernel = complete_kernel(&current);
        kernel
            .insert(record("validation", RecordKind::Evidence, &current))
            .unwrap();
        kernel.accept_evidence(&"validation".into()).unwrap();
        kernel
            .insert(record("must-test", RecordKind::Obligation, &current))
            .unwrap();
        assert_eq!(
            kernel.check_commit(&current),
            Err(KernelError::UndischargedObligations(vec![
                "must-test".into()
            ]))
        );
        assert_eq!(
            kernel.remove_record(&"must-test".into()),
            Err(KernelError::UnresolvedRecordRemoval("must-test".into()))
        );
    }

    #[test]
    fn resolution_evidence_is_dependency_linked_and_invalidates_its_obligation() {
        let current = snapshot("a");
        let mut kernel = complete_kernel(&current);
        kernel
            .insert(record("validation", RecordKind::Evidence, &current))
            .unwrap();
        kernel
            .insert(record("obligation", RecordKind::Obligation, &current))
            .unwrap();
        kernel.accept_evidence(&"validation".into()).unwrap();
        kernel
            .discharge_obligation(&"obligation".into(), &"validation".into())
            .unwrap();

        assert!(kernel.dependency_relations().any(|relation| {
            relation.dependent == "obligation".into()
                && relation.prerequisite == "validation".into()
                && relation.reason == DependencyReason::Validation
        }));
        assert!(matches!(
            kernel.remove_record(&"validation".into()),
            Err(KernelError::RecordHasDependents { .. })
        ));

        let invalidated = kernel
            .invalidate(&"validation".into(), snapshot("b"))
            .unwrap();
        assert_eq!(
            invalidated,
            BTreeSet::from(["obligation".into(), "validation".into()])
        );
        assert!(kernel.validate().is_ok());
        assert!(matches!(
            kernel.check_commit(&current),
            Err(KernelError::StaleRecords(_))
        ));
    }

    #[test]
    fn unknown_for_partial_boundary_cannot_be_erased() {
        let current = snapshot("a");
        let boundary = CoverageBoundary {
            scope: "repository".to_owned(),
            status: CoverageStatus::Partial,
            gaps: vec![CoverageGap {
                id: "dynamic-call-target".to_owned(),
                reason: GapReason::DynamicDispatch,
            }],
        };
        let mut kernel = SemanticKernel::new(current.clone(), boundary).unwrap();
        kernel
            .insert(record("dynamic-call-target", RecordKind::Unknown, &current))
            .unwrap();
        assert_eq!(
            kernel.remove_record(&"dynamic-call-target".into()),
            Err(KernelError::ErasedUnknown(vec![
                "dynamic-call-target".to_owned()
            ]))
        );
        assert!(kernel.record(&"dynamic-call-target".into()).is_some());
    }

    #[test]
    fn rejects_cyclic_derivation_atomically() {
        let current = snapshot("a");
        let mut kernel = complete_kernel(&current);
        for id in ["a", "b", "c"] {
            kernel
                .insert(record(id, RecordKind::DerivedFact, &current))
                .unwrap();
        }
        kernel
            .add_dependency(&"b".into(), &"a".into(), DependencyReason::Derivation)
            .unwrap();
        kernel
            .add_dependency(&"c".into(), &"b".into(), DependencyReason::Derivation)
            .unwrap();
        assert!(matches!(
            kernel.add_dependency(&"a".into(), &"c".into(), DependencyReason::Derivation),
            Err(KernelError::CyclicDerivation(_))
        ));
        assert_eq!(kernel.dependency_relations().count(), 2);
    }

    #[test]
    fn commit_accepts_only_discharged_conflict_free_validated_state() {
        let current = snapshot("a");
        let mut kernel = complete_kernel(&current);
        kernel
            .insert(record("validation", RecordKind::Evidence, &current))
            .unwrap();
        kernel
            .insert(record("obligation", RecordKind::Obligation, &current))
            .unwrap();
        kernel
            .insert(record("conflict", RecordKind::Conflict, &current))
            .unwrap();
        assert_eq!(
            kernel.check_commit(&current),
            Err(KernelError::UndischargedObligations(vec![
                "obligation".into()
            ]))
        );
        kernel.accept_evidence(&"validation".into()).unwrap();
        kernel
            .discharge_obligation(&"obligation".into(), &"validation".into())
            .unwrap();
        assert_eq!(
            kernel.check_commit(&current),
            Err(KernelError::UnresolvedConflicts(vec!["conflict".into()]))
        );
        kernel
            .resolve_conflict(&"conflict".into(), &"validation".into())
            .unwrap();
        assert_eq!(kernel.check_commit(&current), Ok(()));
    }

    #[test]
    fn source_removal_invalidates_dependents_and_inventory_remains_lossy() {
        let current = snapshot("a");
        let mut kernel = complete_kernel(&current);
        let mut observed = record("observed", RecordKind::ObservedFact, &current);
        observed.provenance = vec![Provenance::Source {
            snapshot: current.clone(),
            path: "src/main/kotlin/acme/Price.kt".to_owned(),
            content_hash: "sha256:source".to_owned(),
            start_byte: 20,
            end_byte: 44,
        }];
        kernel.insert(observed).unwrap();
        kernel
            .insert(record("claim", RecordKind::Claim, &current))
            .unwrap();
        kernel
            .add_dependency(
                &"claim".into(),
                &"observed".into(),
                DependencyReason::Justification,
            )
            .unwrap();

        let invalidated = kernel
            .remove_source(
                &current,
                "src/main/kotlin/acme/Price.kt",
                "sha256:source",
                snapshot("b"),
            )
            .unwrap();
        assert_eq!(
            invalidated,
            BTreeSet::from(["claim".into(), "observed".into()])
        );
        assert_eq!(
            kernel.anti_duplication_inventory(),
            AntiDuplicationInventory {
                semantic_records: 2,
                provenance_links: 2,
                source_body_bytes: 0,
                transition_program_bytes: 0,
            }
        );
        let encoded = serde_json::to_string(&kernel).unwrap();
        for forbidden_field in [
            "sourceBody",
            "sourceText",
            "replacement",
            "transitionProgram",
        ] {
            assert!(!encoded.contains(forbidden_field));
        }
    }

    #[test]
    fn current_index_snapshot_has_a_composite_adapter() {
        let existing = Snapshot {
            base_revision: "rev".to_owned(),
            project_model_hash: "model".to_owned(),
            compiler_version: "2.4.10".to_owned(),
            index_snapshot: "index".to_owned(),
            ..Snapshot::default()
        };
        assert_eq!(
            CompositeSnapshot::from_index(&existing, "classpath"),
            CompositeSnapshot {
                base_revision: "rev".to_owned(),
                project_model_hash: "model".to_owned(),
                index_snapshot: "index".to_owned(),
                compiler_version: "2.4.10".to_owned(),
                classpath_hash: "classpath".to_owned(),
            }
        );
    }
}
