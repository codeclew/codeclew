//! Deterministic, fail-closed freshness projection for semantic facts.
//!
//! This is deliberately a small product primitive: callers feed it durable
//! source/build/classpath/compiler observations and invalidations in sequence.
//! It does not discover dependencies and it never turns a stale fact back into
//! `FRESH` without a later observation of that fact.

use crate::canonical;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use thiserror::Error;

pub const FRESHNESS_EVENT_SCHEMA: &str = "freshness-event/0.1";
pub const FRESHNESS_CHECKPOINT_SCHEMA: &str = "freshness-checkpoint/0.1";

/// The four input contours which can make a semantic fact stale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FactDomain {
    Source,
    Build,
    Classpath,
    Compiler,
}

/// A snapshot is carried with every observation and event; hashes are opaque
/// stable values, never source text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FactProvenance {
    pub producer: String,
    pub composite_snapshot_hash: String,
    pub index_snapshot_hash: String,
    pub project_model_hash: String,
    pub classpath_hash: String,
    pub compiler_version: String,
    pub compiler_options_hash: String,
}

/// A fact may depend on facts in any input contour. Dependencies are semantic
/// identifiers, not paths or source excerpts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DependencyFact {
    pub id: String,
    pub domain: FactDomain,
    pub fingerprint: String,
    pub provenance: FactProvenance,
    #[serde(default)]
    pub depends_on: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FreshnessEventKind {
    /// Observes the complete current value of a fact.
    Observed { fact: DependencyFact },
    /// Invalidates a fact and all currently-known transitive dependents.
    Invalidated { fact_id: String, reason: String },
    /// Replaces the entire known fact inventory after a complete authoritative
    /// rebuild. This is the only event allowed to close a persisted gap.
    AuthoritativeReset { reason: String },
}

/// At-least-once envelope. `event_id` is the idempotency key and `sequence`
/// must be strictly contiguous for new events in one projection stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FreshnessEvent {
    pub schema: String,
    pub event_id: String,
    pub sequence: u64,
    pub provenance: FactProvenance,
    pub event: FreshnessEventKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FactFreshness {
    Fresh,
    PartiallyFresh {
        stale_dependencies: Vec<String>,
    },
    Stale {
        invalidated_by: String,
        reason: String,
    },
    Unknown {
        missing_dependencies: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackedFact {
    pub fact: DependencyFact,
    pub freshness: FactFreshness,
    pub last_observed_sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FreshnessCheckpoint {
    pub schema: String,
    pub last_sequence: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sequence_gap: Option<SequenceGap>,
    pub facts: Vec<TrackedFact>,
    /// Stable id -> canonical event hash. Keeping this lets a recovered
    /// projector identify a valid at-least-once replay without reapplying it.
    pub seen_events: BTreeMap<String, String>,
}

/// A stream gap is persistent state, not a transient diagnostic. Until the
/// caller replays a complete stream from a trusted checkpoint, all queries are
/// `UNKNOWN` rather than accidentally serving an old fact as current.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SequenceGap {
    pub expected: u64,
    pub received: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IngestOutcome {
    Applied { sequence: u64 },
    Duplicate { sequence: u64 },
}

/// In-memory deterministic projection. It is intentionally serializable only
/// through [`Self::checkpoint`] so its ordering and deduplication state remain
/// explicit.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FreshnessProjection {
    last_sequence: u64,
    sequence_gap: Option<SequenceGap>,
    facts: BTreeMap<String, TrackedFact>,
    seen_events: BTreeMap<String, String>,
}

impl FreshnessProjection {
    pub fn ingest(&mut self, event: FreshnessEvent) -> Result<IngestOutcome, FreshnessError> {
        event.validate()?;
        let event_hash = canonical::hash(&event)
            .map_err(|error| FreshnessError::Canonical(error.to_string()))?;

        if let Some(previous) = self.seen_events.get(&event.event_id) {
            if previous == &event_hash {
                return Ok(IngestOutcome::Duplicate {
                    sequence: event.sequence,
                });
            }
            return Err(FreshnessError::EventIdConflict {
                event_id: event.event_id,
            });
        }
        if let Some(gap) = &self.sequence_gap
            && !matches!(&event.event, FreshnessEventKind::AuthoritativeReset { .. })
        {
            return Err(FreshnessError::ProjectionGapped(gap.clone()));
        }
        let expected = self
            .last_sequence
            .checked_add(1)
            .ok_or(FreshnessError::SequenceOverflow)?;
        if event.sequence != expected {
            self.sequence_gap = Some(SequenceGap {
                expected,
                received: event.sequence,
            });
            return Err(FreshnessError::OutOfOrder {
                expected,
                received: event.sequence,
            });
        }

        // Mutate a clone. A malformed invalidation therefore cannot leave a
        // half-applied projection behind.
        let mut candidate = self.clone();
        candidate.apply(&event)?;
        candidate.last_sequence = event.sequence;
        candidate.seen_events.insert(event.event_id, event_hash);
        *self = candidate;
        Ok(IngestOutcome::Applied {
            sequence: event.sequence,
        })
    }

    pub fn status(&self, fact_id: &str) -> FactFreshness {
        if let Some(gap) = &self.sequence_gap {
            return FactFreshness::Unknown {
                missing_dependencies: vec![format!(
                    "sequence-gap:{}:{}",
                    gap.expected, gap.received
                )],
            };
        }
        let Some(fact) = self.facts.get(fact_id) else {
            return FactFreshness::Unknown {
                missing_dependencies: vec![fact_id.to_owned()],
            };
        };
        self.status_for(fact, &mut BTreeSet::new())
    }

    pub fn fact(&self, fact_id: &str) -> Option<&TrackedFact> {
        self.facts.get(fact_id)
    }

    pub fn last_sequence(&self) -> u64 {
        self.last_sequence
    }

    pub fn checkpoint(&self) -> FreshnessCheckpoint {
        FreshnessCheckpoint {
            schema: FRESHNESS_CHECKPOINT_SCHEMA.to_owned(),
            last_sequence: self.last_sequence,
            sequence_gap: self.sequence_gap.clone(),
            facts: self.facts.values().cloned().collect(),
            seen_events: self.seen_events.clone(),
        }
    }

    pub fn from_checkpoint(checkpoint: FreshnessCheckpoint) -> Result<Self, FreshnessError> {
        if checkpoint.schema != FRESHNESS_CHECKPOINT_SCHEMA {
            return Err(FreshnessError::UnsupportedCheckpointSchema(
                checkpoint.schema,
            ));
        }
        let mut facts = BTreeMap::new();
        for tracked in checkpoint.facts {
            tracked.fact.validate()?;
            if facts.insert(tracked.fact.id.clone(), tracked).is_some() {
                return Err(FreshnessError::DuplicateFactInCheckpoint);
            }
        }
        for event_id in checkpoint.seen_events.keys() {
            validate_id("event id", event_id)?;
        }
        let projection = Self {
            last_sequence: checkpoint.last_sequence,
            sequence_gap: checkpoint.sequence_gap,
            facts,
            seen_events: checkpoint.seen_events,
        };
        projection.validate_checkpoint()?;
        Ok(projection)
    }

    /// Replays an externally ordered event log. A gap or duplicate id conflict
    /// fails the complete replay rather than silently accepting a partial view.
    pub fn replay(
        events: impl IntoIterator<Item = FreshnessEvent>,
    ) -> Result<Self, FreshnessError> {
        let mut projection = Self::default();
        for event in events {
            projection.ingest(event)?;
        }
        Ok(projection)
    }

    fn apply(&mut self, event: &FreshnessEvent) -> Result<(), FreshnessError> {
        match &event.event {
            FreshnessEventKind::Observed { fact } => {
                fact.validate()?;
                let changed = self
                    .facts
                    .get(&fact.id)
                    .is_some_and(|previous| previous.fact != *fact);
                if changed {
                    self.invalidate_dependents(&fact.id, &event.event_id, "dependency changed");
                }
                self.facts.insert(
                    fact.id.clone(),
                    TrackedFact {
                        fact: fact.clone(),
                        freshness: FactFreshness::Fresh,
                        last_observed_sequence: event.sequence,
                    },
                );
            }
            FreshnessEventKind::Invalidated { fact_id, reason } => {
                validate_id("fact id", fact_id)?;
                validate_id("invalidation reason", reason)?;
                if !self.facts.contains_key(fact_id) {
                    return Err(FreshnessError::UnknownInvalidationTarget(fact_id.clone()));
                }
                self.mark_stale(fact_id, &event.event_id, reason);
                self.invalidate_dependents(fact_id, &event.event_id, reason);
            }
            FreshnessEventKind::AuthoritativeReset { reason } => {
                validate_id("reset reason", reason)?;
                self.facts.clear();
                self.sequence_gap = None;
            }
        }
        Ok(())
    }

    fn invalidate_dependents(&mut self, root: &str, event_id: &str, reason: &str) {
        let mut queue = VecDeque::from([root.to_owned()]);
        let mut visited = BTreeSet::new();
        while let Some(changed) = queue.pop_front() {
            if !visited.insert(changed.clone()) {
                continue;
            }
            let dependents: Vec<String> = self
                .facts
                .values()
                .filter(|candidate| {
                    candidate
                        .fact
                        .depends_on
                        .iter()
                        .any(|dependency| dependency == &changed)
                })
                .map(|candidate| candidate.fact.id.clone())
                .collect();
            for dependent in dependents {
                self.mark_stale(&dependent, event_id, reason);
                queue.push_back(dependent);
            }
        }
    }

    fn mark_stale(&mut self, fact_id: &str, event_id: &str, reason: &str) {
        if let Some(fact) = self.facts.get_mut(fact_id) {
            fact.freshness = FactFreshness::Stale {
                invalidated_by: event_id.to_owned(),
                reason: reason.to_owned(),
            };
        }
    }

    fn status_for(&self, fact: &TrackedFact, visiting: &mut BTreeSet<String>) -> FactFreshness {
        // Direct invalidation always dominates dependency status. This is the
        // central "no active stale fact as fresh" invariant.
        if !matches!(fact.freshness, FactFreshness::Fresh) {
            return fact.freshness.clone();
        }
        if !visiting.insert(fact.fact.id.clone()) {
            return FactFreshness::Unknown {
                missing_dependencies: vec![fact.fact.id.clone()],
            };
        }
        let mut stale = Vec::new();
        let mut missing = Vec::new();
        for dependency in &fact.fact.depends_on {
            match self.facts.get(dependency) {
                None => missing.push(dependency.clone()),
                Some(dependency_fact) => {
                    // A dependency observed later than its dependent cannot
                    // prove the earlier derived fact. Likewise, inputs from a
                    // distinct composite snapshot are not interchangeable.
                    if dependency_fact.last_observed_sequence > fact.last_observed_sequence
                        || !same_input_snapshot(
                            &dependency_fact.fact.provenance,
                            &fact.fact.provenance,
                        )
                    {
                        stale.push(dependency.clone());
                        continue;
                    }
                    match self.status_for(dependency_fact, visiting) {
                        FactFreshness::Fresh => {}
                        FactFreshness::Unknown { .. } => missing.push(dependency.clone()),
                        FactFreshness::PartiallyFresh { .. } | FactFreshness::Stale { .. } => {
                            stale.push(dependency.clone())
                        }
                    }
                }
            }
        }
        visiting.remove(&fact.fact.id);
        stale.sort();
        stale.dedup();
        missing.sort();
        missing.dedup();
        if !missing.is_empty() {
            FactFreshness::Unknown {
                missing_dependencies: missing,
            }
        } else if !stale.is_empty() {
            FactFreshness::PartiallyFresh {
                stale_dependencies: stale,
            }
        } else {
            FactFreshness::Fresh
        }
    }

    fn validate_checkpoint(&self) -> Result<(), FreshnessError> {
        for tracked in self.facts.values() {
            if let FactFreshness::Fresh = tracked.freshness {
                // A missing dependency makes the derived status UNKNOWN; it
                // remains permitted as an observed fact, but is never exposed
                // as FRESH through `status`.
                let _ = self.status(&tracked.fact.id);
            }
        }
        Ok(())
    }
}

fn same_input_snapshot(left: &FactProvenance, right: &FactProvenance) -> bool {
    left.composite_snapshot_hash == right.composite_snapshot_hash
        && left.index_snapshot_hash == right.index_snapshot_hash
        && left.project_model_hash == right.project_model_hash
        && left.classpath_hash == right.classpath_hash
        && left.compiler_version == right.compiler_version
        && left.compiler_options_hash == right.compiler_options_hash
}

impl FreshnessEvent {
    fn validate(&self) -> Result<(), FreshnessError> {
        if self.schema != FRESHNESS_EVENT_SCHEMA {
            return Err(FreshnessError::UnsupportedEventSchema(self.schema.clone()));
        }
        validate_id("event id", &self.event_id)?;
        self.provenance.validate()?;
        match &self.event {
            FreshnessEventKind::Observed { fact } => {
                fact.validate()?;
                if !same_input_snapshot(&self.provenance, &fact.provenance) {
                    return Err(FreshnessError::EventFactProvenanceMismatch {
                        event_id: self.event_id.clone(),
                    });
                }
                Ok(())
            }
            FreshnessEventKind::Invalidated { fact_id, reason } => {
                validate_id("fact id", fact_id)?;
                validate_id("invalidation reason", reason)
            }
            FreshnessEventKind::AuthoritativeReset { reason } => {
                validate_id("reset reason", reason)
            }
        }
    }
}

impl DependencyFact {
    fn validate(&self) -> Result<(), FreshnessError> {
        validate_id("fact id", &self.id)?;
        validate_id("fact fingerprint", &self.fingerprint)?;
        self.provenance.validate()?;
        let mut unique = BTreeSet::new();
        for dependency in &self.depends_on {
            validate_id("dependency id", dependency)?;
            if dependency == &self.id {
                return Err(FreshnessError::SelfDependency(self.id.clone()));
            }
            if !unique.insert(dependency) {
                return Err(FreshnessError::DuplicateDependency {
                    fact_id: self.id.clone(),
                    dependency: dependency.clone(),
                });
            }
        }
        Ok(())
    }
}

impl FactProvenance {
    fn validate(&self) -> Result<(), FreshnessError> {
        for (field, value) in [
            ("producer", &self.producer),
            ("composite snapshot hash", &self.composite_snapshot_hash),
            ("index snapshot hash", &self.index_snapshot_hash),
            ("project model hash", &self.project_model_hash),
            ("classpath hash", &self.classpath_hash),
            ("compiler version", &self.compiler_version),
            ("compiler options hash", &self.compiler_options_hash),
        ] {
            validate_id(field, value)?;
        }
        Ok(())
    }
}

fn validate_id(field: &'static str, value: &str) -> Result<(), FreshnessError> {
    if value.trim().is_empty() || value.chars().any(char::is_whitespace) {
        return Err(FreshnessError::InvalidIdentifier {
            field,
            value: value.to_owned(),
        });
    }
    Ok(())
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum FreshnessError {
    #[error("unsupported freshness event schema {0}")]
    UnsupportedEventSchema(String),
    #[error("unsupported freshness checkpoint schema {0}")]
    UnsupportedCheckpointSchema(String),
    #[error("invalid {field}: {value:?}")]
    InvalidIdentifier { field: &'static str, value: String },
    #[error("event sequence overflow")]
    SequenceOverflow,
    #[error("out-of-order event: expected sequence {expected}, received {received}")]
    OutOfOrder { expected: u64, received: u64 },
    #[error("projection has a persistent sequence gap: expected {0:?}")]
    ProjectionGapped(SequenceGap),
    #[error("event id {event_id} was replayed with different content")]
    EventIdConflict { event_id: String },
    #[error("event {event_id} and observed fact have different input provenance")]
    EventFactProvenanceMismatch { event_id: String },
    #[error("cannot invalidate unknown fact {0}")]
    UnknownInvalidationTarget(String),
    #[error("fact {0} depends on itself")]
    SelfDependency(String),
    #[error("fact {fact_id} names dependency {dependency} more than once")]
    DuplicateDependency { fact_id: String, dependency: String },
    #[error("checkpoint contains a duplicate fact")]
    DuplicateFactInCheckpoint,
    #[error("cannot canonicalize freshness event: {0}")]
    Canonical(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provenance() -> FactProvenance {
        FactProvenance {
            producer: "kotlin-worker-2.1".into(),
            composite_snapshot_hash: "snapshot-1".into(),
            index_snapshot_hash: "index-1".into(),
            project_model_hash: "project-1".into(),
            classpath_hash: "classpath-1".into(),
            compiler_version: "2.1.21".into(),
            compiler_options_hash: "compiler-1".into(),
        }
    }

    fn fact(
        id: &str,
        domain: FactDomain,
        fingerprint: &str,
        depends_on: &[&str],
    ) -> DependencyFact {
        DependencyFact {
            id: id.into(),
            domain,
            fingerprint: fingerprint.into(),
            provenance: provenance(),
            depends_on: depends_on.iter().map(|value| (*value).into()).collect(),
        }
    }

    fn observed(sequence: u64, id: &str, fact: DependencyFact) -> FreshnessEvent {
        FreshnessEvent {
            schema: FRESHNESS_EVENT_SCHEMA.into(),
            event_id: id.into(),
            sequence,
            provenance: provenance(),
            event: FreshnessEventKind::Observed { fact },
        }
    }

    fn invalidated(sequence: u64, id: &str, fact_id: &str) -> FreshnessEvent {
        FreshnessEvent {
            schema: FRESHNESS_EVENT_SCHEMA.into(),
            event_id: id.into(),
            sequence,
            provenance: provenance(),
            event: FreshnessEventKind::Invalidated {
                fact_id: fact_id.into(),
                reason: "input-changed".into(),
            },
        }
    }

    fn authoritative_reset(sequence: u64, id: &str) -> FreshnessEvent {
        FreshnessEvent {
            schema: FRESHNESS_EVENT_SCHEMA.into(),
            event_id: id.into(),
            sequence,
            provenance: provenance(),
            event: FreshnessEventKind::AuthoritativeReset {
                reason: "full-rebuild".into(),
            },
        }
    }

    #[test]
    fn tracks_freshness_across_all_input_contours() {
        let events = vec![
            observed(
                1,
                "e-source",
                fact("source:Order", FactDomain::Source, "s1", &[]),
            ),
            observed(
                2,
                "e-build",
                fact("build:main", FactDomain::Build, "b1", &[]),
            ),
            observed(
                3,
                "e-classpath",
                fact("classpath:jackson", FactDomain::Classpath, "c1", &[]),
            ),
            observed(
                4,
                "e-compiler",
                fact("compiler:k2", FactDomain::Compiler, "k1", &[]),
            ),
            observed(
                5,
                "e-semantic",
                fact(
                    "semantic:OrderMapper",
                    FactDomain::Source,
                    "m1",
                    &[
                        "source:Order",
                        "build:main",
                        "classpath:jackson",
                        "compiler:k2",
                    ],
                ),
            ),
        ];
        let projection = FreshnessProjection::replay(events).unwrap();
        assert_eq!(
            projection.status("semantic:OrderMapper"),
            FactFreshness::Fresh
        );
    }

    #[test]
    fn invalidation_never_leaves_dependents_fresh() {
        let mut projection = FreshnessProjection::replay(vec![
            observed(1, "e1", fact("source:Order", FactDomain::Source, "s1", &[])),
            observed(
                2,
                "e2",
                fact(
                    "semantic:OrderMapper",
                    FactDomain::Source,
                    "m1",
                    &["source:Order"],
                ),
            ),
        ])
        .unwrap();
        projection
            .ingest(invalidated(3, "e3", "source:Order"))
            .unwrap();
        assert!(matches!(
            projection.status("source:Order"),
            FactFreshness::Stale { .. }
        ));
        assert!(matches!(
            projection.status("semantic:OrderMapper"),
            FactFreshness::Stale { .. }
        ));
    }

    #[test]
    fn same_event_is_idempotent_but_conflicting_duplicate_fails_closed() {
        let event = observed(1, "e1", fact("source:Order", FactDomain::Source, "s1", &[]));
        let mut projection = FreshnessProjection::default();
        assert_eq!(
            projection.ingest(event.clone()).unwrap(),
            IngestOutcome::Applied { sequence: 1 }
        );
        assert_eq!(
            projection.ingest(event.clone()).unwrap(),
            IngestOutcome::Duplicate { sequence: 1 }
        );
        let mut conflicting = event;
        conflicting.sequence = 2;
        assert_eq!(
            projection.ingest(conflicting),
            Err(FreshnessError::EventIdConflict {
                event_id: "e1".into()
            })
        );
        assert_eq!(projection.last_sequence(), 1);
    }

    #[test]
    fn gap_makes_previously_fresh_facts_unknown_until_full_replay() {
        let mut projection = FreshnessProjection::replay(vec![observed(
            1,
            "e1",
            fact("source:Order", FactDomain::Source, "s1", &[]),
        )])
        .unwrap();
        assert_eq!(projection.status("source:Order"), FactFreshness::Fresh);
        let event = observed(3, "e3", fact("build:main", FactDomain::Build, "b1", &[]));
        assert_eq!(
            projection.ingest(event),
            Err(FreshnessError::OutOfOrder {
                expected: 2,
                received: 3
            })
        );
        assert_eq!(projection.last_sequence(), 1);
        assert_eq!(
            projection.status("source:Order"),
            FactFreshness::Unknown {
                missing_dependencies: vec!["sequence-gap:2:3".into()]
            }
        );
        assert!(matches!(
            projection.ingest(observed(
                2,
                "e2",
                fact("build:main", FactDomain::Build, "b1", &[])
            )),
            Err(FreshnessError::ProjectionGapped(_))
        ));
        let restored = FreshnessProjection::from_checkpoint(projection.checkpoint()).unwrap();
        assert_eq!(restored, projection);
        assert_eq!(
            restored.status("source:Order"),
            FactFreshness::Unknown {
                missing_dependencies: vec!["sequence-gap:2:3".into()]
            }
        );
        let mut rebuilding = restored;
        rebuilding
            .ingest(authoritative_reset(2, "authoritative-reset"))
            .unwrap();
        assert_eq!(
            rebuilding.status("source:Order"),
            FactFreshness::Unknown {
                missing_dependencies: vec!["source:Order".into()]
            }
        );
        assert_eq!(rebuilding.last_sequence(), 2);
    }

    #[test]
    fn replay_has_a_deterministic_fail_closed_result_for_a_gap() {
        let events = vec![
            observed(1, "e1", fact("source:Order", FactDomain::Source, "s1", &[])),
            observed(3, "e3", fact("build:main", FactDomain::Build, "b1", &[])),
        ];
        let expected = Err(FreshnessError::OutOfOrder {
            expected: 2,
            received: 3,
        });
        assert_eq!(FreshnessProjection::replay(events.clone()), expected);
        assert_eq!(FreshnessProjection::replay(events), expected);
    }

    #[test]
    fn missing_or_stale_dependencies_are_not_reported_fresh() {
        let mut projection = FreshnessProjection::replay(vec![observed(
            1,
            "e1",
            fact(
                "semantic:OrderMapper",
                FactDomain::Source,
                "m1",
                &["source:Order"],
            ),
        )])
        .unwrap();
        assert_eq!(
            projection.status("semantic:OrderMapper"),
            FactFreshness::Unknown {
                missing_dependencies: vec!["source:Order".into()]
            }
        );
        projection
            .ingest(observed(
                2,
                "e2",
                fact("source:Order", FactDomain::Source, "s1", &[]),
            ))
            .unwrap();
        assert_eq!(
            projection.status("semantic:OrderMapper"),
            FactFreshness::PartiallyFresh {
                stale_dependencies: vec!["source:Order".into()]
            }
        );
        projection
            .ingest(observed(
                3,
                "e3",
                fact(
                    "semantic:OrderMapper",
                    FactDomain::Source,
                    "m2",
                    &["source:Order"],
                ),
            ))
            .unwrap();
        assert_eq!(
            projection.status("semantic:OrderMapper"),
            FactFreshness::Fresh
        );
        projection
            .ingest(invalidated(4, "e4", "source:Order"))
            .unwrap();
        assert!(matches!(
            projection.status("semantic:OrderMapper"),
            FactFreshness::Stale { .. }
        ));
    }

    #[test]
    fn dependency_from_a_different_snapshot_is_never_fresh() {
        let mut different_snapshot = provenance();
        different_snapshot.composite_snapshot_hash = "snapshot-2".into();
        let dependency = fact("source:Order", FactDomain::Source, "s1", &[]);
        let mut derived = fact(
            "semantic:OrderMapper",
            FactDomain::Source,
            "m1",
            &["source:Order"],
        );
        derived.provenance = different_snapshot;
        let mut derived_event = observed(2, "e2", derived.clone());
        derived_event.provenance = derived.provenance.clone();
        let projection =
            FreshnessProjection::replay(vec![observed(1, "e1", dependency), derived_event])
                .unwrap();
        assert_eq!(
            projection.status("semantic:OrderMapper"),
            FactFreshness::PartiallyFresh {
                stale_dependencies: vec!["source:Order".into()]
            }
        );
    }

    #[test]
    fn event_and_fact_must_name_the_same_input_snapshot() {
        let mut fact = fact("source:Order", FactDomain::Source, "s1", &[]);
        fact.provenance.composite_snapshot_hash = "snapshot-2".into();
        let event = observed(1, "e1", fact);
        assert_eq!(
            FreshnessProjection::default().ingest(event),
            Err(FreshnessError::EventFactProvenanceMismatch {
                event_id: "e1".into()
            })
        );
    }

    #[test]
    fn checkpoint_round_trip_and_replay_are_deterministic() {
        let events = vec![
            observed(1, "e1", fact("source:Order", FactDomain::Source, "s1", &[])),
            observed(
                2,
                "e2",
                fact(
                    "semantic:OrderMapper",
                    FactDomain::Source,
                    "m1",
                    &["source:Order"],
                ),
            ),
            invalidated(3, "e3", "source:Order"),
        ];
        let replayed = FreshnessProjection::replay(events.clone()).unwrap();
        let restored = FreshnessProjection::from_checkpoint(replayed.checkpoint()).unwrap();
        assert_eq!(restored, replayed);
        assert_eq!(
            canonical::hash(&restored.checkpoint()).unwrap(),
            canonical::hash(&FreshnessProjection::replay(events).unwrap().checkpoint()).unwrap()
        );
    }
}
