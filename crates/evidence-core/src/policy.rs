use thiserror::Error;

use std::collections::BTreeSet;

use crate::protocol::{
    Approximation, Boundary, BoundaryConsequence, ClaimResult, ClaimSpec, Coverage, Enumeration,
    EvidenceFact, ObligationGraph, ObligationStatus, RelationAssertion, Truth,
};
use crate::{ContractRegistry, Validate, ValidationErrors};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObligationClosureContract {
    pub capability_digest: String,
    pub specification_digest: String,
    pub obligation_ids: BTreeSet<String>,
}

#[derive(Debug, Error)]
pub enum PolicyError {
    #[error(transparent)]
    Invalid(#[from] ValidationErrors),
    #[error("invalid obligation closure contract digest: {0}")]
    InvalidClosureDigest(String),
    #[error("obligation closure binding mismatch")]
    ClosureBindingMismatch,
    #[error("obligation closure mismatch: missing {missing:?}, extra {extra:?}")]
    ObligationClosureMismatch {
        missing: Vec<String>,
        extra: Vec<String>,
    },
}

/// Fail-closed policy over evidence metadata. The policy compares exact grade
/// categories and operation contracts; it never interprets language payloads.
pub struct DecisionPolicy<'a> {
    registry: &'a ContractRegistry,
}

impl<'a> DecisionPolicy<'a> {
    pub fn new(registry: &'a ContractRegistry) -> Self {
        Self { registry }
    }

    pub fn evaluate_claim(
        &self,
        claim: &ClaimSpec,
        facts: &[EvidenceFact],
        aggregate_coverage: &Coverage,
    ) -> Result<ClaimResult, PolicyError> {
        claim.validate()?;
        aggregate_coverage.validate()?;
        if !coverage_admissible(claim, aggregate_coverage) {
            return Ok(ClaimResult::Unknown);
        }

        let Some(expected) = claim.claim.as_ref() else {
            return Ok(ClaimResult::Unknown);
        };
        let expected_truth = Truth::try_from(expected.truth).unwrap_or(Truth::Unknown);
        let mut saw_supporting = false;
        let mut saw_unknown = false;

        for fact in facts {
            fact.validate()?;
            let Some(actual) = fact.assertion.as_ref() else {
                saw_unknown = true;
                continue;
            };
            if !same_predicate(expected, actual) {
                continue;
            }
            let Some(descriptor) = self.registry.descriptor(&fact.capability_descriptor_digest)
            else {
                saw_unknown = true;
                continue;
            };
            let Some(key) = descriptor.key.as_ref() else {
                saw_unknown = true;
                continue;
            };
            if !claim.accepted_grades.contains(&key.grade)
                || fact
                    .coverage
                    .as_ref()
                    .is_none_or(|coverage| !coverage_admissible(claim, coverage))
            {
                saw_unknown = true;
                continue;
            }
            match Truth::try_from(actual.truth).unwrap_or(Truth::Unknown) {
                Truth::Unknown | Truth::Unspecified => saw_unknown = true,
                actual_truth if actual_truth == expected_truth => saw_supporting = true,
                _ => return Ok(ClaimResult::Violated),
            }
        }

        Ok(if saw_supporting && !saw_unknown {
            ClaimResult::Satisfied
        } else {
            ClaimResult::Unknown
        })
    }

    pub fn evaluate_obligation_graph(
        &self,
        graph: &ObligationGraph,
    ) -> Result<ClaimResult, PolicyError> {
        graph.validate()?;
        let mandatory = graph.obligations.iter().filter(|item| item.mandatory);
        let statuses = mandatory
            .map(|item| {
                ObligationStatus::try_from(item.status).unwrap_or(ObligationStatus::Unknown)
            })
            .collect::<Vec<_>>();
        Ok(if statuses.contains(&ObligationStatus::Violated) {
            ClaimResult::Violated
        } else if statuses.contains(&ObligationStatus::Unsupported) {
            ClaimResult::Unsupported
        } else if statuses
            .iter()
            .all(|status| *status == ObligationStatus::Satisfied)
        {
            ClaimResult::Satisfied
        } else {
            ClaimResult::Unknown
        })
    }

    /// Evaluate only if the provider-owned closure contains exactly the
    /// preregistered obligation ids. Missing and injected obligations both fail
    /// closed because either changes the meaning of the claimed closure.
    pub fn evaluate_obligation_graph_exact(
        &self,
        graph: &ObligationGraph,
        expected: &ObligationClosureContract,
    ) -> Result<ClaimResult, PolicyError> {
        graph.validate()?;
        for digest in [&expected.capability_digest, &expected.specification_digest] {
            if !is_sha256(digest) {
                return Err(PolicyError::InvalidClosureDigest(digest.clone()));
            }
        }
        if graph.closure_capability_digest != expected.capability_digest
            || graph.closure_specification_digest != expected.specification_digest
        {
            return Err(PolicyError::ClosureBindingMismatch);
        }
        let actual = graph
            .obligations
            .iter()
            .map(|obligation| obligation.obligation_id.clone())
            .collect::<BTreeSet<_>>();
        if actual != expected.obligation_ids {
            return Err(PolicyError::ObligationClosureMismatch {
                missing: expected
                    .obligation_ids
                    .difference(&actual)
                    .cloned()
                    .collect(),
                extra: actual
                    .difference(&expected.obligation_ids)
                    .cloned()
                    .collect(),
            });
        }
        self.evaluate_obligation_graph(graph)
    }

    pub fn evaluate_impact_coverage(
        &self,
        coverage: &Coverage,
        unknown_boundaries: &[Boundary],
    ) -> Result<ClaimResult, PolicyError> {
        coverage.validate()?;
        Ok(
            if coverage.enumeration == Enumeration::CompleteInScope as i32
                && coverage.approximation != Approximation::Heuristic as i32
                && unknown_boundaries.is_empty()
                && !has_proof_invalid_boundary(coverage)
            {
                ClaimResult::Satisfied
            } else {
                ClaimResult::Unknown
            },
        )
    }
}

fn is_sha256(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

fn same_predicate(left: &RelationAssertion, right: &RelationAssertion) -> bool {
    left.relation == right.relation
        && left.operands == right.operands
        && left.language_payload == right.language_payload
}

fn coverage_admissible(claim: &ClaimSpec, coverage: &Coverage) -> bool {
    if claim.reject_proof_invalid_boundary && has_proof_invalid_boundary(coverage) {
        return false;
    }
    let enumeration = Enumeration::try_from(coverage.enumeration).ok();
    let required = Enumeration::try_from(claim.required_enumeration).ok();
    let enumeration_ok = match required {
        Some(Enumeration::CompleteInScope) => enumeration == Some(Enumeration::CompleteInScope),
        Some(Enumeration::Partial) => matches!(
            enumeration,
            Some(Enumeration::CompleteInScope | Enumeration::Partial)
        ),
        Some(Enumeration::Unknown) => enumeration.is_some(),
        _ => false,
    };
    enumeration_ok
        && claim
            .accepted_approximations
            .contains(&coverage.approximation)
}

fn has_proof_invalid_boundary(coverage: &Coverage) -> bool {
    coverage
        .boundaries
        .iter()
        .any(|boundary| boundary.consequence == BoundaryConsequence::ProofInvalid as i32)
}
