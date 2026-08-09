use crate::semantic_kernel::{
    CoverageStatus, EvidencePurpose, Freshness, RecordId, RecordKind, ResolutionState,
    SemanticKernel, SemanticRecord, SemanticRelation, SemanticValue, Soundness,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub const SEMANTIC_GOAL_SCHEMA: &str = "semantic-goal/0.1";
pub const CHANGE_GRAPH_SCHEMA: &str = "change-graph/0.1";
pub const GOAL_PROOF_SCHEMA: &str = "semantic-goal-proof/0.1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum GoalFamily {
    MapEdgeWithContext,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Preservation {
    Order,
    Cardinality,
    Laziness,
    Effects,
    Nullability,
    ConsumerContract,
    Abi,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SemanticGoal {
    pub schema: String,
    pub family: GoalFamily,
    pub base_revision: String,
    pub element_type: String,
    pub context_type: String,
    pub result_type: String,
    pub context_evaluation: ContextEvaluation,
    pub preserve: BTreeSet<Preservation>,
    #[serde(default)]
    pub business_choices: BTreeMap<String, String>,
}

impl SemanticGoal {
    pub fn map_edge_with_context(
        base_revision: impl Into<String>,
        element_type: impl Into<String>,
        context_type: impl Into<String>,
    ) -> Self {
        Self {
            schema: SEMANTIC_GOAL_SCHEMA.into(),
            family: GoalFamily::MapEdgeWithContext,
            base_revision: base_revision.into(),
            element_type: element_type.into(),
            context_type: context_type.into(),
            result_type: "SAME_AS_ELEMENT".into(),
            context_evaluation: ContextEvaluation::OncePerRegion,
            preserve: [
                Preservation::Order,
                Preservation::Cardinality,
                Preservation::Laziness,
                Preservation::Effects,
                Preservation::Nullability,
                Preservation::ConsumerContract,
            ]
            .into_iter()
            .collect(),
            business_choices: BTreeMap::new(),
        }
    }

    fn validate(&self) -> Result<(), Refusal> {
        if self.schema != SEMANTIC_GOAL_SCHEMA {
            return Err(Refusal::InvalidGoalSchema);
        }
        if self.base_revision.is_empty()
            || self.element_type.is_empty()
            || self.context_type.is_empty()
        {
            return Err(Refusal::IncompleteGoal);
        }
        if self.result_type != "SAME_AS_ELEMENT" {
            return Err(Refusal::ResultTypeNotPreserved);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ContextEvaluation {
    OncePerRegion,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ObligationKind {
    BindUniqueContextProducer,
    BindUniqueTransformer,
    BindUniqueValueEdge,
    TypeAssignable,
    IntroduceOnce,
    MapValueEdge,
    PreserveOrder,
    PreserveCardinality,
    PreserveLaziness,
    PreserveEffects,
    PreserveNullability,
    PreserveConsumerContract,
    PreserveAbi,
    RequireOracle,
    MustRefuseOnBoundary,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DischargeStatus {
    Proved,
    Unproved,
    Refused,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ChangeObligation {
    pub id: String,
    pub kind: ObligationKind,
    pub subject: Vec<String>,
    pub depends_on: Vec<String>,
    pub evidence: Vec<String>,
    pub status: DischargeStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ChangeGraph {
    pub schema: String,
    pub goal_schema: String,
    pub obligations: Vec<ChangeObligation>,
}

impl ChangeGraph {
    pub fn validate_closure(&self) -> Result<(), Refusal> {
        let ids: BTreeSet<_> = self
            .obligations
            .iter()
            .map(|item| item.id.as_str())
            .collect();
        if ids.len() != self.obligations.len() {
            return Err(Refusal::InvalidObligationGraph);
        }
        if self
            .obligations
            .iter()
            .flat_map(|item| item.depends_on.iter())
            .any(|dependency| !ids.contains(dependency.as_str()))
        {
            return Err(Refusal::InvalidObligationGraph);
        }

        let mut indegree: BTreeMap<&str, usize> = ids.iter().map(|id| (*id, 0)).collect();
        let mut outgoing: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for item in &self.obligations {
            for dependency in &item.depends_on {
                *indegree
                    .get_mut(item.id.as_str())
                    .expect("known obligation") += 1;
                outgoing
                    .entry(dependency.as_str())
                    .or_default()
                    .push(item.id.as_str());
            }
        }
        let mut queue: VecDeque<_> = indegree
            .iter()
            .filter_map(|(id, degree)| (*degree == 0).then_some(*id))
            .collect();
        let mut visited = 0;
        while let Some(id) = queue.pop_front() {
            visited += 1;
            for next in outgoing.get(id).into_iter().flatten() {
                let degree = indegree.get_mut(next).expect("known obligation");
                *degree -= 1;
                if *degree == 0 {
                    queue.push_back(next);
                }
            }
        }
        if visited != self.obligations.len() {
            return Err(Refusal::CyclicObligationGraph);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SemanticCandidate {
    pub symbol: String,
    pub evidence_ref: RecordId,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EvidencePredicate {
    TypeAssignable,
    PlacementDominatesUses,
    ContextEvaluatedOnce,
    OrderPreserved,
    CardinalityPreserved,
    LazinessPreserved,
    EffectsPreserved,
    NullabilityPreserved,
    ConsumerContractPreserved,
    AbiPreserved,
    BehavioralOracleAvailable,
}

impl EvidencePredicate {
    fn semantic_subject(self) -> &'static str {
        match self {
            Self::TypeAssignable => "binding.type-assignable",
            Self::PlacementDominatesUses => "binding.placement-dominates-uses",
            Self::ContextEvaluatedOnce => "binding.context-evaluated-once",
            Self::OrderPreserved => "binding.order-preserved",
            Self::CardinalityPreserved => "binding.cardinality-preserved",
            Self::LazinessPreserved => "binding.laziness-preserved",
            Self::EffectsPreserved => "binding.effects-preserved",
            Self::NullabilityPreserved => "binding.nullability-preserved",
            Self::ConsumerContractPreserved => "binding.consumer-contract-preserved",
            Self::AbiPreserved => "binding.abi-preserved",
            Self::BehavioralOracleAvailable => "binding.behavioral-oracle-available",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SemanticBoundary {
    ExternalCall,
    DynamicDispatch,
    DependencyInjection,
    Reflection,
    Transaction,
    Lifecycle,
    Suspend,
    UnknownEffect,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BindingEvidence {
    pub snapshot_revision: String,
    pub context_producers: Vec<SemanticCandidate>,
    pub transformers: Vec<SemanticCandidate>,
    pub value_edges: Vec<SemanticCandidate>,
    #[serde(default)]
    pub boundaries: Vec<SemanticBoundary>,
    pub type_assignable: bool,
    pub placement_dominates_uses: bool,
    pub context_evaluated_once: bool,
    pub order_preserved: bool,
    pub cardinality_preserved: bool,
    pub laziness_preserved: bool,
    pub effects_preserved: bool,
    pub nullability_preserved: bool,
    pub consumer_contract_preserved: bool,
    pub abi_preserved: bool,
    pub behavioral_oracle_available: bool,
}

impl BindingEvidence {
    fn predicate_value(&self, predicate: EvidencePredicate) -> bool {
        match predicate {
            EvidencePredicate::TypeAssignable => self.type_assignable,
            EvidencePredicate::PlacementDominatesUses => self.placement_dominates_uses,
            EvidencePredicate::ContextEvaluatedOnce => self.context_evaluated_once,
            EvidencePredicate::OrderPreserved => self.order_preserved,
            EvidencePredicate::CardinalityPreserved => self.cardinality_preserved,
            EvidencePredicate::LazinessPreserved => self.laziness_preserved,
            EvidencePredicate::EffectsPreserved => self.effects_preserved,
            EvidencePredicate::NullabilityPreserved => self.nullability_preserved,
            EvidencePredicate::ConsumerContractPreserved => self.consumer_contract_preserved,
            EvidencePredicate::AbiPreserved => self.abi_preserved,
            EvidencePredicate::BehavioralOracleAvailable => self.behavioral_oracle_available,
        }
    }
}

/// Semantic binding inputs plus the kernel records that justify every
/// compiler-derived predicate. A proof cannot be produced from unreferenced
/// booleans: all consumed predicates must resolve to current, sound records in
/// the same composite snapshot.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KernelBindingEvidence {
    pub facts: BindingEvidence,
    pub predicate_evidence: BTreeMap<EvidencePredicate, RecordId>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProofStatus {
    Bound,
    Ambiguous,
    Refused,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Refusal {
    InvalidGoalSchema,
    IncompleteGoal,
    ResultTypeNotPreserved,
    SnapshotMismatch,
    UnsupportedBoundary,
    TypeNotAssignable,
    PlacementNotDominating,
    ContextEvaluationNotOnce,
    PreservationNotProved,
    MissingBehavioralOracle,
    InvalidObligationGraph,
    CyclicObligationGraph,
    IncompleteKernelCoverage,
    MissingKernelEvidence,
    StaleKernelEvidence,
    InsufficientKernelEvidence,
    InvalidKernelEvidence,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GoalProof {
    pub schema: String,
    pub goal_fingerprint: String,
    pub kernel_snapshot_fingerprint: String,
    pub status: ProofStatus,
    pub bindings: BTreeMap<String, String>,
    pub change_graph: ChangeGraph,
    pub ambiguities: BTreeMap<String, Vec<String>>,
    pub boundaries: Vec<SemanticBoundary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refusal: Option<Refusal>,
}

impl GoalProof {
    pub fn is_complete_for_goal(&self, goal: &SemanticGoal, kernel: &SemanticKernel) -> bool {
        let expected_fingerprint = crate::canonical::hash(goal).ok();
        let expected_snapshot_fingerprint = crate::canonical::hash(kernel.snapshot()).ok();
        let required_bindings = ["contextProducer", "transformer", "valueEdge"];
        let mut required_kinds = vec![
            ObligationKind::BindUniqueContextProducer,
            ObligationKind::BindUniqueTransformer,
            ObligationKind::BindUniqueValueEdge,
            ObligationKind::TypeAssignable,
            ObligationKind::IntroduceOnce,
            ObligationKind::MapValueEdge,
            ObligationKind::RequireOracle,
            ObligationKind::MustRefuseOnBoundary,
        ];
        required_kinds.extend(goal.preserve.iter().map(preservation_obligation));
        goal.validate().is_ok()
            && kernel.validate().is_ok()
            && kernel.coverage().status == CoverageStatus::Complete
            && kernel.snapshot().base_revision == goal.base_revision
            && self.schema == GOAL_PROOF_SCHEMA
            && self.goal_fingerprint == expected_fingerprint.unwrap_or_default()
            && self.kernel_snapshot_fingerprint == expected_snapshot_fingerprint.unwrap_or_default()
            && self.change_graph.schema == CHANGE_GRAPH_SCHEMA
            && self.change_graph.goal_schema == goal.schema
            && self.status == ProofStatus::Bound
            && self.refusal.is_none()
            && self.boundaries.is_empty()
            && self.ambiguities.is_empty()
            && self.bindings.len() == required_bindings.len()
            && required_bindings.iter().all(|role| {
                self.bindings
                    .get(*role)
                    .is_some_and(|value| !value.is_empty())
            })
            && self.change_graph.validate_closure().is_ok()
            && self.change_graph.obligations.len() == required_kinds.len()
            && required_kinds.iter().all(|kind| {
                self.change_graph
                    .obligations
                    .iter()
                    .filter(|item| item.kind == *kind)
                    .count()
                    == 1
            })
            && self.change_graph.obligations.iter().all(|item| {
                item.status == DischargeStatus::Proved
                    && !item.id.is_empty()
                    && !item.subject.is_empty()
                    && !item.evidence.is_empty()
            })
            && proof_payload_matches_kernel(self, goal, kernel)
    }
}

fn proof_payload_matches_kernel(
    proof: &GoalProof,
    goal: &SemanticGoal,
    kernel: &SemanticKernel,
) -> bool {
    let obligation = |kind: ObligationKind| {
        proof
            .change_graph
            .obligations
            .iter()
            .find(|item| item.kind == kind)
    };
    let context = match proof.bindings.get("contextProducer") {
        Some(value) => value,
        None => return false,
    };
    let transformer = match proof.bindings.get("transformer") {
        Some(value) => value,
        None => return false,
    };
    let edge = match proof.bindings.get("valueEdge") {
        Some(value) => value,
        None => return false,
    };

    if proof.change_graph.obligations.iter().any(|item| {
        item.evidence
            .iter()
            .any(|id| usable_kernel_record(kernel, &RecordId(id.clone())).is_err())
    }) {
        return false;
    }

    let binding_specs = [
        (ObligationKind::BindUniqueContextProducer, context.as_str()),
        (ObligationKind::BindUniqueTransformer, transformer.as_str()),
        (ObligationKind::BindUniqueValueEdge, edge.as_str()),
    ];
    if binding_specs.iter().any(|(kind, symbol)| {
        obligation(kind.clone()).is_none_or(|item| {
            item.subject != [(*symbol).to_owned()]
                || !obligation_has_atom(
                    item,
                    kernel,
                    symbol,
                    SemanticRelation::Exists,
                    SemanticValue::Boolean(true),
                )
        })
    }) {
        return false;
    }

    let edge_subjects = [transformer.clone(), edge.clone()];
    for kind in [ObligationKind::TypeAssignable, ObligationKind::MapValueEdge] {
        if obligation(kind).is_none_or(|item| {
            item.subject != edge_subjects
                || !obligation_has_candidate(item, kernel, transformer)
                || !obligation_has_candidate(item, kernel, edge)
        }) {
            return false;
        }
    }
    if obligation(ObligationKind::IntroduceOnce).is_none_or(|item| {
        item.subject != [context.clone()] || !obligation_has_candidate(item, kernel, context)
    }) || obligation(ObligationKind::RequireOracle).is_none_or(|item| {
        item.subject != [edge.clone()] || !obligation_has_candidate(item, kernel, edge)
    }) || obligation(ObligationKind::MustRefuseOnBoundary).is_none_or(|item| {
        item.subject != [edge.clone()] || !obligation_has_candidate(item, kernel, edge)
    }) {
        return false;
    }

    let predicate_specs = [
        (
            ObligationKind::TypeAssignable,
            EvidencePredicate::TypeAssignable,
        ),
        (
            ObligationKind::IntroduceOnce,
            EvidencePredicate::PlacementDominatesUses,
        ),
        (
            ObligationKind::IntroduceOnce,
            EvidencePredicate::ContextEvaluatedOnce,
        ),
        (
            ObligationKind::RequireOracle,
            EvidencePredicate::BehavioralOracleAvailable,
        ),
    ];
    if predicate_specs.iter().any(|(kind, predicate)| {
        obligation(kind.clone()).is_none_or(|item| {
            !obligation_has_atom(
                item,
                kernel,
                predicate.semantic_subject(),
                SemanticRelation::Satisfies,
                SemanticValue::Boolean(true),
            )
        })
    }) {
        return false;
    }
    for preservation in &goal.preserve {
        let kind = preservation_obligation(preservation);
        let predicate = preservation_predicate(preservation);
        if obligation(kind).is_none_or(|item| {
            item.subject != [edge.clone()]
                || !obligation_has_candidate(item, kernel, edge)
                || !obligation_has_atom(
                    item,
                    kernel,
                    predicate.semantic_subject(),
                    SemanticRelation::Satisfies,
                    SemanticValue::Boolean(true),
                )
        }) {
            return false;
        }
    }
    true
}

fn obligation_has_candidate(
    obligation: &ChangeObligation,
    kernel: &SemanticKernel,
    symbol: &str,
) -> bool {
    obligation_has_atom(
        obligation,
        kernel,
        symbol,
        SemanticRelation::Exists,
        SemanticValue::Boolean(true),
    )
}

fn obligation_has_atom(
    obligation: &ChangeObligation,
    kernel: &SemanticKernel,
    subject: &str,
    relation: SemanticRelation,
    value: SemanticValue,
) -> bool {
    obligation.evidence.iter().any(|id| {
        kernel.record(&RecordId(id.clone())).is_some_and(|record| {
            record.atom.subject == subject
                && record.atom.relation == relation
                && record.atom.value == value
        })
    })
}

pub fn bind_goal(
    goal: &SemanticGoal,
    evidence: &KernelBindingEvidence,
    kernel: &SemanticKernel,
) -> GoalProof {
    if let Err(refusal) = validate_kernel_evidence(goal, evidence, kernel) {
        return refused(refusal, evidence.facts.boundaries.clone());
    }
    bind_facts(
        goal,
        &evidence.facts,
        &evidence.predicate_evidence,
        crate::canonical::hash(kernel.snapshot()).unwrap_or_default(),
    )
}

fn bind_facts(
    goal: &SemanticGoal,
    evidence: &BindingEvidence,
    predicate_evidence: &BTreeMap<EvidencePredicate, RecordId>,
    kernel_snapshot_fingerprint: String,
) -> GoalProof {
    if let Err(refusal) = goal.validate() {
        return refused(refusal, evidence.boundaries.clone());
    }
    if goal.base_revision != evidence.snapshot_revision {
        return refused(Refusal::SnapshotMismatch, evidence.boundaries.clone());
    }
    if !evidence.boundaries.is_empty() {
        return refused(Refusal::UnsupportedBoundary, evidence.boundaries.clone());
    }
    if !evidence.type_assignable {
        return refused(Refusal::TypeNotAssignable, vec![]);
    }
    if !evidence.placement_dominates_uses {
        return refused(Refusal::PlacementNotDominating, vec![]);
    }
    if !evidence.context_evaluated_once {
        return refused(Refusal::ContextEvaluationNotOnce, vec![]);
    }
    if !evidence.behavioral_oracle_available {
        return refused(Refusal::MissingBehavioralOracle, vec![]);
    }
    if !preservations_hold(goal, evidence) {
        return refused(Refusal::PreservationNotProved, vec![]);
    }

    let candidates = [
        ("contextProducer", &evidence.context_producers),
        ("transformer", &evidence.transformers),
        ("valueEdge", &evidence.value_edges),
    ];
    let ambiguities: BTreeMap<_, _> = candidates
        .iter()
        .filter(|(_, values)| values.len() != 1)
        .map(|(role, values)| {
            (
                (*role).to_owned(),
                values
                    .iter()
                    .map(|candidate| candidate.symbol.clone())
                    .collect(),
            )
        })
        .collect();
    if !ambiguities.is_empty() {
        return GoalProof {
            schema: GOAL_PROOF_SCHEMA.into(),
            goal_fingerprint: crate::canonical::hash(goal).unwrap_or_default(),
            kernel_snapshot_fingerprint,
            status: ProofStatus::Ambiguous,
            bindings: BTreeMap::new(),
            change_graph: ChangeGraph {
                schema: CHANGE_GRAPH_SCHEMA.into(),
                goal_schema: goal.schema.clone(),
                obligations: vec![],
            },
            ambiguities,
            boundaries: vec![],
            refusal: None,
        };
    }

    let bindings: BTreeMap<_, _> = candidates
        .iter()
        .map(|(role, values)| ((*role).to_owned(), values[0].symbol.clone()))
        .collect();
    let change_graph = build_change_graph(goal, evidence, predicate_evidence);
    debug_assert!(change_graph.validate_closure().is_ok());
    GoalProof {
        schema: GOAL_PROOF_SCHEMA.into(),
        goal_fingerprint: crate::canonical::hash(goal).unwrap_or_default(),
        kernel_snapshot_fingerprint,
        status: ProofStatus::Bound,
        bindings,
        change_graph,
        ambiguities: BTreeMap::new(),
        boundaries: vec![],
        refusal: None,
    }
}

fn validate_kernel_evidence(
    goal: &SemanticGoal,
    evidence: &KernelBindingEvidence,
    kernel: &SemanticKernel,
) -> Result<(), Refusal> {
    kernel
        .validate()
        .map_err(|_| Refusal::InvalidKernelEvidence)?;
    if kernel.coverage().status != CoverageStatus::Complete {
        return Err(Refusal::IncompleteKernelCoverage);
    }
    if goal.base_revision != kernel.snapshot().base_revision
        || evidence.facts.snapshot_revision != kernel.snapshot().base_revision
    {
        return Err(Refusal::SnapshotMismatch);
    }

    let required_predicates = [
        EvidencePredicate::TypeAssignable,
        EvidencePredicate::PlacementDominatesUses,
        EvidencePredicate::ContextEvaluatedOnce,
        EvidencePredicate::OrderPreserved,
        EvidencePredicate::CardinalityPreserved,
        EvidencePredicate::LazinessPreserved,
        EvidencePredicate::EffectsPreserved,
        EvidencePredicate::NullabilityPreserved,
        EvidencePredicate::ConsumerContractPreserved,
        EvidencePredicate::AbiPreserved,
        EvidencePredicate::BehavioralOracleAvailable,
    ];
    for candidate in evidence
        .facts
        .context_producers
        .iter()
        .chain(&evidence.facts.transformers)
        .chain(&evidence.facts.value_edges)
    {
        let record = usable_kernel_record(kernel, &candidate.evidence_ref)?;
        if record.atom.subject != candidate.symbol
            || record.atom.relation != SemanticRelation::Exists
            || record.atom.value != SemanticValue::Boolean(true)
        {
            return Err(Refusal::InvalidKernelEvidence);
        }
    }
    for predicate in required_predicates {
        let record_id = evidence
            .predicate_evidence
            .get(&predicate)
            .ok_or(Refusal::MissingKernelEvidence)?;
        let record = usable_kernel_record(kernel, record_id)?;
        if record.atom.subject != predicate.semantic_subject()
            || record.atom.relation != SemanticRelation::Satisfies
            || record.atom.value
                != SemanticValue::Boolean(evidence.facts.predicate_value(predicate))
        {
            return Err(Refusal::InvalidKernelEvidence);
        }
    }
    Ok(())
}

fn usable_kernel_record<'a>(
    kernel: &'a SemanticKernel,
    record_id: &RecordId,
) -> Result<&'a SemanticRecord, Refusal> {
    let record = kernel
        .record(record_id)
        .ok_or(Refusal::MissingKernelEvidence)?;
    if record.freshness != Freshness::Current || !record.is_current_at(kernel.snapshot()) {
        return Err(Refusal::StaleKernelEvidence);
    }
    if record.soundness != Soundness::Sound {
        return Err(Refusal::InsufficientKernelEvidence);
    }
    let usable = matches!(
        record.kind,
        RecordKind::ObservedFact | RecordKind::DerivedFact | RecordKind::Invariant
    ) || matches!(
        (&record.kind, &record.resolution),
        (
            RecordKind::Evidence,
            ResolutionState::EvidenceAccepted {
                purpose: EvidencePurpose::Validation | EvidencePurpose::Explanation
            }
        )
    );
    if !usable {
        return Err(Refusal::InsufficientKernelEvidence);
    }
    Ok(record)
}

fn preservation_obligation(value: &Preservation) -> ObligationKind {
    match value {
        Preservation::Order => ObligationKind::PreserveOrder,
        Preservation::Cardinality => ObligationKind::PreserveCardinality,
        Preservation::Laziness => ObligationKind::PreserveLaziness,
        Preservation::Effects => ObligationKind::PreserveEffects,
        Preservation::Nullability => ObligationKind::PreserveNullability,
        Preservation::ConsumerContract => ObligationKind::PreserveConsumerContract,
        Preservation::Abi => ObligationKind::PreserveAbi,
    }
}

fn preservation_predicate(value: &Preservation) -> EvidencePredicate {
    match value {
        Preservation::Order => EvidencePredicate::OrderPreserved,
        Preservation::Cardinality => EvidencePredicate::CardinalityPreserved,
        Preservation::Laziness => EvidencePredicate::LazinessPreserved,
        Preservation::Effects => EvidencePredicate::EffectsPreserved,
        Preservation::Nullability => EvidencePredicate::NullabilityPreserved,
        Preservation::ConsumerContract => EvidencePredicate::ConsumerContractPreserved,
        Preservation::Abi => EvidencePredicate::AbiPreserved,
    }
}

fn preservations_hold(goal: &SemanticGoal, evidence: &BindingEvidence) -> bool {
    goal.preserve.iter().all(|item| match item {
        Preservation::Order => evidence.order_preserved,
        Preservation::Cardinality => evidence.cardinality_preserved,
        Preservation::Laziness => evidence.laziness_preserved,
        Preservation::Effects => evidence.effects_preserved,
        Preservation::Nullability => evidence.nullability_preserved,
        Preservation::ConsumerContract => evidence.consumer_contract_preserved,
        Preservation::Abi => evidence.abi_preserved,
    })
}

fn build_change_graph(
    goal: &SemanticGoal,
    evidence: &BindingEvidence,
    predicate_evidence: &BTreeMap<EvidencePredicate, RecordId>,
) -> ChangeGraph {
    let context = &evidence.context_producers[0];
    let transformer = &evidence.transformers[0];
    let edge = &evidence.value_edges[0];
    let mut obligations = vec![
        obligation(
            "bind-context",
            ObligationKind::BindUniqueContextProducer,
            vec![context],
            vec![],
        ),
        obligation(
            "bind-transformer",
            ObligationKind::BindUniqueTransformer,
            vec![transformer],
            vec![],
        ),
        obligation(
            "bind-edge",
            ObligationKind::BindUniqueValueEdge,
            vec![edge],
            vec![],
        ),
        with_predicate_evidence(
            obligation(
                "type-assignable",
                ObligationKind::TypeAssignable,
                vec![transformer, edge],
                vec!["bind-transformer", "bind-edge"],
            ),
            predicate_evidence,
            &[EvidencePredicate::TypeAssignable],
        ),
        with_predicate_evidence(
            obligation(
                "introduce-once",
                ObligationKind::IntroduceOnce,
                vec![context],
                vec!["bind-context", "bind-edge"],
            ),
            predicate_evidence,
            &[
                EvidencePredicate::ContextEvaluatedOnce,
                EvidencePredicate::PlacementDominatesUses,
            ],
        ),
        obligation(
            "map-edge",
            ObligationKind::MapValueEdge,
            vec![transformer, edge],
            vec!["type-assignable", "introduce-once"],
        ),
        with_predicate_evidence(
            obligation(
                "require-oracle",
                ObligationKind::RequireOracle,
                vec![edge],
                vec!["map-edge"],
            ),
            predicate_evidence,
            &[EvidencePredicate::BehavioralOracleAvailable],
        ),
        obligation(
            "boundary-check",
            ObligationKind::MustRefuseOnBoundary,
            vec![edge],
            vec!["bind-edge"],
        ),
    ];
    for preservation in &goal.preserve {
        let suffix = format!(
            "preserve-{}",
            serde_json::to_string(preservation)
                .unwrap()
                .trim_matches('"')
                .to_ascii_lowercase()
        );
        obligations.push(with_predicate_evidence(
            obligation(
                &suffix,
                preservation_obligation(preservation),
                vec![edge],
                vec!["map-edge"],
            ),
            predicate_evidence,
            &[preservation_predicate(preservation)],
        ));
    }
    ChangeGraph {
        schema: CHANGE_GRAPH_SCHEMA.into(),
        goal_schema: goal.schema.clone(),
        obligations,
    }
}

fn obligation(
    id: &str,
    kind: ObligationKind,
    subjects: Vec<&SemanticCandidate>,
    depends_on: Vec<&str>,
) -> ChangeObligation {
    ChangeObligation {
        id: id.into(),
        kind,
        subject: subjects.iter().map(|item| item.symbol.clone()).collect(),
        depends_on: depends_on.into_iter().map(str::to_owned).collect(),
        evidence: subjects
            .iter()
            .map(|item| item.evidence_ref.0.clone())
            .collect(),
        status: DischargeStatus::Proved,
    }
}

fn with_predicate_evidence(
    mut obligation: ChangeObligation,
    predicate_evidence: &BTreeMap<EvidencePredicate, RecordId>,
    predicates: &[EvidencePredicate],
) -> ChangeObligation {
    obligation
        .evidence
        .extend(predicates.iter().map(|predicate| {
            predicate_evidence
                .get(predicate)
                .expect("kernel validation requires every predicate record")
                .0
                .clone()
        }));
    obligation.evidence.sort();
    obligation.evidence.dedup();
    obligation
}

fn refused(reason: Refusal, boundaries: Vec<SemanticBoundary>) -> GoalProof {
    GoalProof {
        schema: GOAL_PROOF_SCHEMA.into(),
        goal_fingerprint: String::new(),
        kernel_snapshot_fingerprint: String::new(),
        status: ProofStatus::Refused,
        bindings: BTreeMap::new(),
        change_graph: ChangeGraph {
            schema: CHANGE_GRAPH_SCHEMA.into(),
            goal_schema: SEMANTIC_GOAL_SCHEMA.into(),
            obligations: vec![],
        },
        ambiguities: BTreeMap::new(),
        boundaries,
        refusal: Some(reason),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::semantic_kernel::{
        CompositeSnapshot, CoverageBoundary, Provenance, SemanticAtom, SemanticRecord,
        SemanticRelation, SemanticValue, ValidityInterval,
    };

    fn snapshot(revision: &str) -> CompositeSnapshot {
        CompositeSnapshot {
            base_revision: revision.into(),
            project_model_hash: format!("model-{revision}"),
            index_snapshot: format!("index-{revision}"),
            compiler_version: "2.1.0".into(),
            classpath_hash: format!("classpath-{revision}"),
        }
    }

    fn record(
        id: &RecordId,
        snapshot: &CompositeSnapshot,
        subject: &str,
        relation: SemanticRelation,
        value: SemanticValue,
    ) -> SemanticRecord {
        SemanticRecord {
            id: id.clone(),
            kind: RecordKind::ObservedFact,
            atom: SemanticAtom {
                subject: subject.into(),
                relation,
                value,
            },
            provenance: vec![Provenance::IndexFact {
                snapshot: snapshot.clone(),
                fact_key: id.0.clone(),
                fact_hash: format!("hash-{}", id.0),
            }],
            validity: ValidityInterval::current(snapshot.clone()),
            soundness: Soundness::Sound,
            resolution: ResolutionState::NotApplicable,
            freshness: Freshness::Current,
        }
    }

    fn candidate(symbol: &str) -> SemanticCandidate {
        SemanticCandidate {
            symbol: symbol.into(),
            evidence_ref: format!("fact:{symbol}").as_str().into(),
        }
    }

    fn evidence() -> KernelBindingEvidence {
        let predicates = [
            EvidencePredicate::TypeAssignable,
            EvidencePredicate::PlacementDominatesUses,
            EvidencePredicate::ContextEvaluatedOnce,
            EvidencePredicate::OrderPreserved,
            EvidencePredicate::CardinalityPreserved,
            EvidencePredicate::LazinessPreserved,
            EvidencePredicate::EffectsPreserved,
            EvidencePredicate::NullabilityPreserved,
            EvidencePredicate::ConsumerContractPreserved,
            EvidencePredicate::AbiPreserved,
            EvidencePredicate::BehavioralOracleAvailable,
        ];
        KernelBindingEvidence {
            facts: BindingEvidence {
                snapshot_revision: "base".into(),
                context_producers: vec![candidate("loadContext")],
                transformers: vec![candidate("decorate")],
                value_edges: vec![candidate("producer->consumer")],
                boundaries: vec![],
                type_assignable: true,
                placement_dominates_uses: true,
                context_evaluated_once: true,
                order_preserved: true,
                cardinality_preserved: true,
                laziness_preserved: true,
                effects_preserved: true,
                nullability_preserved: true,
                consumer_contract_preserved: true,
                abi_preserved: true,
                behavioral_oracle_available: true,
            },
            predicate_evidence: predicates
                .into_iter()
                .map(|predicate| {
                    let id = format!("proof:{predicate:?}").as_str().into();
                    (predicate, id)
                })
                .collect(),
        }
    }

    fn kernel_for(evidence: &KernelBindingEvidence) -> SemanticKernel {
        kernel_for_with_degraded_predicate(evidence, None)
    }

    fn kernel_for_with_degraded_predicate(
        evidence: &KernelBindingEvidence,
        degraded: Option<EvidencePredicate>,
    ) -> SemanticKernel {
        let current = snapshot("base");
        let mut kernel =
            SemanticKernel::new(current.clone(), CoverageBoundary::complete("goal-surface"))
                .unwrap();
        for candidate in evidence
            .facts
            .context_producers
            .iter()
            .chain(&evidence.facts.transformers)
            .chain(&evidence.facts.value_edges)
        {
            kernel
                .insert(record(
                    &candidate.evidence_ref,
                    &current,
                    &candidate.symbol,
                    SemanticRelation::Exists,
                    SemanticValue::Boolean(true),
                ))
                .unwrap();
        }
        for (predicate, id) in &evidence.predicate_evidence {
            let mut item = record(
                id,
                &current,
                predicate.semantic_subject(),
                SemanticRelation::Satisfies,
                SemanticValue::Boolean(evidence.facts.predicate_value(*predicate)),
            );
            if degraded == Some(*predicate) {
                item.soundness = Soundness::Conservative;
            }
            kernel.insert(item).unwrap();
        }
        kernel
    }

    fn prove(goal: &SemanticGoal, evidence: &KernelBindingEvidence) -> GoalProof {
        bind_goal(goal, evidence, &kernel_for(evidence))
    }

    #[test]
    fn unique_semantic_binding_builds_complete_change_graph() {
        let facts = evidence();
        let goal = SemanticGoal::map_edge_with_context("base", "Item", "Context");
        let kernel = kernel_for(&facts);
        let proof = bind_goal(&goal, &facts, &kernel);
        assert_eq!(proof.status, ProofStatus::Bound);
        assert!(proof.is_complete_for_goal(&goal, &kernel));
        assert!(proof.change_graph.obligations.len() >= 14);
        assert!(
            proof
                .change_graph
                .obligations
                .iter()
                .all(|item| !item.evidence.is_empty())
        );
        assert!(proof.change_graph.obligations.iter().all(|item| {
            item.evidence
                .iter()
                .all(|id| kernel_for(&facts).record(&RecordId(id.clone())).is_some())
        }));

        let mut vacuous = proof.clone();
        vacuous.bindings.clear();
        vacuous.change_graph.obligations.clear();
        assert!(!vacuous.is_complete_for_goal(&goal, &kernel));
        let different_goal = SemanticGoal::map_edge_with_context("base", "Item", "OtherContext");
        assert!(!proof.is_complete_for_goal(&different_goal, &kernel));

        let mut forged = proof.clone();
        for value in forged.bindings.values_mut() {
            *value = "forged".into();
        }
        for item in &mut forged.change_graph.obligations {
            item.subject = vec!["forged".into()];
            item.evidence = vec!["missing".into()];
        }
        assert!(!forged.is_complete_for_goal(&goal, &kernel));

        let mut stale_kernel = kernel.clone();
        let changed = facts.predicate_evidence[&EvidencePredicate::TypeAssignable].clone();
        stale_kernel
            .invalidate(&changed, snapshot("moved"))
            .unwrap();
        assert!(!proof.is_complete_for_goal(&goal, &stale_kernel));
    }

    #[test]
    fn ambiguity_never_produces_obligations_or_bindings() {
        let mut facts = evidence();
        facts.facts.transformers.push(candidate("decorateOther"));
        let proof = prove(
            &SemanticGoal::map_edge_with_context("base", "Item", "Context"),
            &facts,
        );
        assert_eq!(proof.status, ProofStatus::Ambiguous);
        assert!(proof.bindings.is_empty());
        assert!(proof.change_graph.obligations.is_empty());
    }

    #[test]
    fn unsupported_boundary_and_missing_oracle_refuse() {
        let goal = SemanticGoal::map_edge_with_context("base", "Item", "Context");
        let mut facts = evidence();
        facts
            .facts
            .boundaries
            .push(SemanticBoundary::DependencyInjection);
        assert_eq!(
            prove(&goal, &facts).refusal,
            Some(Refusal::UnsupportedBoundary)
        );
        let mut facts = evidence();
        facts.facts.behavioral_oracle_available = false;
        assert_eq!(
            prove(&goal, &facts).refusal,
            Some(Refusal::MissingBehavioralOracle)
        );
    }

    #[test]
    fn preservation_failure_and_stale_snapshot_refuse() {
        let goal = SemanticGoal::map_edge_with_context("base", "Item", "Context");
        let mut facts = evidence();
        facts.facts.laziness_preserved = false;
        assert_eq!(
            prove(&goal, &facts).refusal,
            Some(Refusal::PreservationNotProved)
        );
        let mut facts = evidence();
        facts.facts.snapshot_revision = "moved".into();
        assert_eq!(
            prove(&goal, &facts).refusal,
            Some(Refusal::SnapshotMismatch)
        );
    }

    #[test]
    fn missing_or_non_sound_kernel_records_refuse_before_binding() {
        let goal = SemanticGoal::map_edge_with_context("base", "Item", "Context");
        let mut facts = evidence();
        facts
            .predicate_evidence
            .remove(&EvidencePredicate::EffectsPreserved);
        let kernel = kernel_for(&facts);
        assert_eq!(
            bind_goal(&goal, &facts, &kernel).refusal,
            Some(Refusal::MissingKernelEvidence)
        );

        let facts = evidence();
        let kernel =
            kernel_for_with_degraded_predicate(&facts, Some(EvidencePredicate::EffectsPreserved));
        assert_eq!(
            bind_goal(&goal, &facts, &kernel).refusal,
            Some(Refusal::InsufficientKernelEvidence)
        );
    }

    #[test]
    fn semantically_mismatched_predicate_record_cannot_prove_an_obligation() {
        let goal = SemanticGoal::map_edge_with_context("base", "Item", "Context");
        let facts = evidence();
        let current = snapshot("base");
        let mut kernel = kernel_for(&facts);
        let wrong_id = facts.predicate_evidence[&EvidencePredicate::EffectsPreserved].clone();
        kernel.remove_record(&wrong_id).unwrap();
        kernel
            .insert(record(
                &wrong_id,
                &current,
                EvidencePredicate::OrderPreserved.semantic_subject(),
                SemanticRelation::Satisfies,
                SemanticValue::Boolean(true),
            ))
            .unwrap();

        let proof = bind_goal(&goal, &facts, &kernel);
        assert_eq!(proof.status, ProofStatus::Refused);
        assert_eq!(proof.refusal, Some(Refusal::InvalidKernelEvidence));
        assert!(proof.change_graph.obligations.is_empty());
    }

    #[test]
    fn invalidated_kernel_fact_makes_the_vertical_slice_fail_closed() {
        let goal = SemanticGoal::map_edge_with_context("base", "Item", "Context");
        let facts = evidence();
        let mut kernel = kernel_for(&facts);
        let changed = facts.predicate_evidence[&EvidencePredicate::TypeAssignable].clone();
        kernel.invalidate(&changed, snapshot("moved")).unwrap();

        let proof = bind_goal(&goal, &facts, &kernel);
        assert_eq!(proof.status, ProofStatus::Refused);
        assert_eq!(proof.refusal, Some(Refusal::StaleKernelEvidence));
        assert!(proof.change_graph.obligations.is_empty());
    }

    #[test]
    fn cyclic_or_missing_obligation_dependency_is_invalid() {
        let mut graph = ChangeGraph {
            schema: CHANGE_GRAPH_SCHEMA.into(),
            goal_schema: SEMANTIC_GOAL_SCHEMA.into(),
            obligations: vec![ChangeObligation {
                id: "a".into(),
                kind: ObligationKind::RequireOracle,
                subject: vec![],
                depends_on: vec!["missing".into()],
                evidence: vec![],
                status: DischargeStatus::Unproved,
            }],
        };
        assert_eq!(
            graph.validate_closure(),
            Err(Refusal::InvalidObligationGraph)
        );
        graph.obligations[0].depends_on = vec!["a".into()];
        assert_eq!(
            graph.validate_closure(),
            Err(Refusal::CyclicObligationGraph)
        );
    }

    #[test]
    fn serialized_goal_has_no_source_patch_or_graph_id_escape_hatch() {
        let json = serde_json::to_string(&SemanticGoal::map_edge_with_context(
            "base", "Item", "Context",
        ))
        .unwrap();
        for forbidden in ["sourceText", "replacement", "graphId", "regex", "EditIR"] {
            assert!(!json.contains(forbidden), "goal leaked {forbidden}: {json}");
        }
    }
}
