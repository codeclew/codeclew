//! Closed author proposals and deterministic checks, separate from meaning acceptance.
use super::{
    bindings, bytes, check, digest, invalid,
    model::*,
    render,
    store::{self, Repository},
    work::{self, Work},
};
use crate::error::{ClewError, ErrorCode};
use clap::Subcommand;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

#[derive(Debug, Subcommand)]
pub enum Command {
    Submit {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        work: String,
        #[arg(long)]
        input: PathBuf,
    },
    Show {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        proposal: String,
        #[arg(long)]
        cursor: Option<String>,
        #[arg(long,default_value_t=20,value_parser=clap::value_parser!(u32).range(1..=100))]
        limit: u32,
    },
    /// Publish checked local authoring while retaining UNASSESSED meaning review.
    Publish {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        proposal: String,
        #[arg(long, required = true)]
        unassessed: bool,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Proposal {
    pub schema: String,
    pub operations: Vec<ProposedOperation>,
    #[serde(default)]
    pub gaps: BTreeMap<String, String>,
    #[serde(default)]
    pub uncertainties: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProposedOperation {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "provided_visuals"
    )]
    pub visuals: Option<Vec<super::visuals::ProposedVisual>>,
    pub entrypoint: String,
    pub title: String,
    pub summary: Claim,
    #[serde(default)]
    pub assessment: Option<ProposedAssessment>,
    #[serde(default)]
    pub dataflow: Option<super::dataflow::ProposedGraph>,
    pub steps: Vec<Step>,
    #[serde(default)]
    pub contracts: Vec<Contract>,
    #[serde(default)]
    pub participants: Vec<ParticipantInput>,
    #[serde(default)]
    pub explanation: Vec<Claim>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ParticipantInput {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub service: Option<String>,
}
fn provided_visuals<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Vec<super::visuals::ProposedVisual>>, D::Error> {
    Vec::deserialize(deserializer).map(Some)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProposedAssessment {
    pub outcome: String,
    pub period: String,
    #[serde(default)]
    pub proposed_correction: Option<Claim>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Claim {
    pub text: String,
    pub evidence: Vec<String>,
    #[serde(default)]
    pub checks: Vec<Assertion>,
    #[serde(default)]
    pub uncertainty: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Assertion {
    pub kind: String,
    pub evidence: String,
    pub field: String,
    pub expected: Value,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Step {
    pub kind: String,
    pub meaning: Claim,
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub to: Option<String>,
    #[serde(default)]
    pub interaction: Option<String>,
    #[serde(default)]
    pub children: Vec<Step>,
    #[serde(default)]
    pub otherwise: Vec<Step>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Contract {
    pub title: String,
    pub kind: String,
    pub rows: Vec<ContractRow>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContractRow {
    pub label: String,
    pub description: Claim,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Artifact {
    pub schema: String,
    pub id: String,
    pub work: String,
    pub input: Proposal,
    pub narrative: Option<Narrative>,
    pub status: String,
    pub diagnostics: Vec<Value>,
    pub claims: BTreeMap<String, Value>,
    pub read_digest: String,
    pub influence: BTreeMap<String, String>,
    pub meaning_review: String,
}

// The exact Work identity already retains the full conservative influence map.
// Keep one authority for those bytes instead of copying them into every proposal.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredArtifact {
    schema: String,
    artifact: Artifact,
    influence_digest: String,
}

impl StoredArtifact {
    fn from_runtime(mut artifact: Artifact) -> Result<Self, ClewError> {
        let influence_digest = digest(&std::mem::take(&mut artifact.influence))?;
        artifact.id.clear();
        let mut stored = Self {
            schema: "codeclew-documentation-proposal-record/1.0".into(),
            artifact,
            influence_digest,
        };
        stored.artifact.id = digest(&stored)?[7..].into();
        Ok(stored)
    }

    fn validate_identity(&self, id: &str) -> Result<(), ClewError> {
        let mut canonical = self.clone();
        canonical.artifact.id.clear();
        if self.schema != "codeclew-documentation-proposal-record/1.0"
            || self.artifact.schema != "codeclew-documentation-proposal-result/1.0"
            || !self.artifact.influence.is_empty()
            || self.artifact.id != id
            || &digest(&canonical)?[7..] != id
        {
            return Err(invalid("proposal record digest or schema is invalid"));
        }
        Ok(())
    }

    fn hydrate(mut self, influence: BTreeMap<String, String>) -> Result<Artifact, ClewError> {
        if digest(&influence)? != self.influence_digest {
            return Err(invalid("proposal influence does not match its saved work"));
        }
        self.artifact.influence = influence;
        Ok(self.artifact)
    }
}
fn stable(scope: &str, slot: &str) -> Result<String, ClewError> {
    Ok(format!("claim-{}", &digest(&(scope, slot))?[7..27]))
}
fn file(id: &str) -> Result<String, ClewError> {
    if id.len() != 64 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(invalid("invalid proposal identity"));
    }
    Ok(format!(".codeclew/proposals/{id}.json"))
}
pub fn load(repo: &Repository, id: &str) -> Result<Artifact, ClewError> {
    let stored: StoredArtifact = store::read(&repo.path(&file(id)?)?, 4 * 1024 * 1024)?;
    stored.validate_identity(id)?;
    let influence = work::load_influence(repo, &stored.artifact.work)?;
    stored.hydrate(influence)
}
pub fn run(command: Command) -> Result<Value, ClewError> {
    match command {
        Command::Publish {
            root,
            proposal,
            unassessed: _,
        } => {
            let repo = Repository::open(&root)?;
            let artifact = load(&repo, &proposal)?;
            let work = work::load(&repo, &artifact.work)?;
            current(&repo, &work)?;
            let reads = digest(&work::read_state(&repo, &work.id)?)?;
            if reads != artifact.read_digest {
                return Err(invalid(
                    "work reads changed; submit the proposal again before publication",
                ));
            }
            let versions = super::review::unassessed_versions(&work, &artifact, &reads)?;
            let narrative = artifact
                .narrative
                .ok_or_else(|| invalid("proposal has no canonical content"))?;
            let mut result =
                render::publish_reviewed(&repo, narrative, versions, work.snapshot.as_deref())?;
            result["meaningReview"] = json!("UNASSESSED");
            result["proposal"] = json!(proposal);
            Ok(result)
        }
        Command::Submit { root, work, input } => submit(
            &Repository::open(&root)?,
            &work,
            store::read(&input, store::MAX_RECORD)?,
        ),
        Command::Show {
            root,
            proposal,
            cursor,
            limit,
        } => show(
            &Repository::open(&root)?,
            &proposal,
            cursor.as_deref(),
            limit as usize,
        ),
    }
}

/// Validate the selected evidence contract and admitted external inputs.
/// Snapshot work stays historical; its publication does not claim current sources.
pub fn current(repo: &Repository, work: &Work) -> Result<(), ClewError> {
    let snapshot = work.snapshot.as_deref().ok_or_else(|| ClewError::new(
        ErrorCode::StaleRequiresReslice,
        "DOCS_REINDEX_REQUIRED: Work requires a saved snapshot; prepare new Work from current-format evidence",
    ))?;
    let now = check::Check::load_snapshot(repo, snapshot)?;
    if now.input_digest != repo.input_digest()? || digest(&now)? != digest(&work.checked)? {
        return Err(ClewError::new(
            ErrorCode::StaleRequiresReslice,
            "saved work evidence or documentation declarations changed",
        ));
    }
    if now
        .services
        .iter()
        .map(|(id, e)| (id, &e.revision))
        .collect::<BTreeMap<_, _>>()
        != work
            .checked
            .services
            .iter()
            .map(|(id, e)| (id, &e.revision))
            .collect::<BTreeMap<_, _>>()
        || now.context_digest != work.checked.context_digest
        || now.input_digest != work.checked.input_digest
        || work::capture_inputs(repo, &work.request)? != work.external_inputs
    {
        return Err(ClewError::new(
            ErrorCode::StaleRequiresReslice,
            "work inputs changed; prepare new work and review the changed evidence",
        ));
    }
    let retained =
        bindings::baseline(repo)?.and_then(|(_, b)| b.narratives.get(&work.subject).cloned());
    let generated_after_preparation = work.retained.is_none()
        && retained
            .as_ref()
            .is_some_and(render::is_generated_placeholder);
    if retained != work.retained && !generated_after_preparation {
        return Err(ClewError::new(
            ErrorCode::WwConflict,
            "published content changed after work preparation",
        ));
    }
    Ok(())
}

struct Builder<'a> {
    work: &'a Work,
    received: BTreeSet<String>,
    claims: BTreeMap<String, Value>,
    diagnostics: Vec<Value>,
    events: usize,
}

/// The proposal validator's reference capabilities are also exposed as read
/// metadata.  Keep these predicates pure so that advisory roles cannot drift
/// from the checks which authorize a proposal.
pub(super) fn evidence_reference_allowed(handle: &work::Handle) -> bool {
    matches!(
        handle.kind.as_str(),
        "DEPENDENCY" | "NOTE" | "ENTRYPOINT" | "SOURCE"
    )
}

pub(super) fn operation_reference_allowed(work: &Work, handle: &work::Handle) -> bool {
    if !matches!(
        handle.kind.as_str(),
        "ENTRYPOINT" | "SECTION" | "NOTE" | "PROCESS_ROOT"
    ) {
        return false;
    }
    work.request.entrypoint.as_ref().is_none_or(|entrypoint| {
        entrypoint == &handle.id
            || (handle.kind == "NOTE"
                && handle
                    .id
                    .strip_prefix("note:")
                    .is_some_and(|id| entrypoint == &super::notes::root(id)))
    })
}

pub(super) fn gap_reference_allowed(handle: &work::Handle) -> bool {
    matches!(
        handle.kind.as_str(),
        "ENTRYPOINT" | "SECTION" | "NOTE" | "PROCESS_ROOT"
    )
}

impl Builder<'_> {
    fn handle(&self, reference: &str) -> Result<&work::Handle, ClewError> {
        if !self.received.contains(reference) {
            if !self.work.handles.contains_key(reference) {
                return Err(invalid(format!("unknown work reference {reference}")));
            }
            return Err(invalid(format!(
                "evidence {reference} was not supplied by a recorded work read; expand it first"
            )));
        }
        self.work
            .handles
            .get(reference)
            .ok_or_else(|| invalid(format!("unknown work reference {reference}")))
    }
    fn evidence(&self, references: &[String]) -> Result<(Vec<String>, Vec<String>), ClewError> {
        if references.is_empty() || references.len() > 32 {
            return Err(invalid("claim needs 1..32 recorded evidence references"));
        }
        let mut deps = BTreeSet::new();
        let mut sources = BTreeSet::new();
        for reference in references {
            let handle = self.handle(reference)?;
            if !evidence_reference_allowed(handle) {
                return Err(invalid("unsupported evidence reference"));
            }
            match handle.kind.as_str() {
                "DEPENDENCY" | "NOTE" => {
                    let observation = &self.work.checked.dependencies[&handle.id];
                    deps.insert(handle.id.clone());
                    sources.extend(observation.source_ids.iter().cloned());
                }
                "ENTRYPOINT" => {
                    let entry = self
                        .work
                        .checked
                        .services
                        .values()
                        .flat_map(|s| s.entrypoints.iter())
                        .find(|e| e.id == handle.id)
                        .ok_or_else(|| invalid("entrypoint disappeared"))?;
                    deps.extend(entry.dependency_ids.iter().cloned());
                    sources.extend(entry.source_ids.iter().cloned());
                }
                "SOURCE" => {
                    sources.insert(handle.id.clone());
                    deps.extend(
                        self.work
                            .checked
                            .dependencies
                            .values()
                            .filter(|d| {
                                d.source_ids.contains(&handle.id)
                                    && self.work.influence.contains_key(&d.id)
                            })
                            .map(|d| d.id.clone()),
                    );
                }
                _ => return Err(invalid("unsupported evidence reference")),
            }
        }
        if deps.len() > 128
            || (sources.is_empty()
                && !deps.iter().all(|id| {
                    matches!(
                        self.work.checked.dependencies[id].kind.as_str(),
                        "NOTE_ASSOCIATION" | "DOMAIN_ENTITY" | "VIEW_DEFINITION"
                    )
                }))
            || sources.len() > 32
        {
            return Err(invalid(
                "claim evidence exceeds canonical bounds or has no source",
            ));
        }
        Ok((deps.into_iter().collect(), sources.into_iter().collect()))
    }
    fn claim(&mut self, scope: &str, slot: &str, claim: &Claim) -> Result<Fragment, ClewError> {
        let id = stable(scope, slot)?;
        if claim.text.trim().is_empty()
            || claim.text.len() > 8192
            || claim.checks.len() > 16
            || claim
                .uncertainty
                .as_ref()
                .is_some_and(|s| s.trim().is_empty() || s.len() > 2048)
        {
            return Err(invalid(format!(
                "{slot}: claim text, uncertainty or checks exceed bounds"
            )));
        }
        let (deps, sources) = self
            .evidence(&claim.evidence)
            .map_err(|e| invalid(format!("{slot}: {}", e.message)))?;
        let mut results = Vec::new();
        for (index, assertion) in claim.checks.iter().enumerate() {
            if !claim.evidence.contains(&assertion.evidence) {
                return Err(invalid(format!(
                    "{slot}: assertion evidence must belong to its claim"
                )));
            }
            let handle = self.handle(&assertion.evidence)?;
            let observed = self.work.checked.dependencies.get(&handle.id);
            let supported = assertion.kind == "factEquals"
                && observed.is_some_and(|d| match d.kind.as_str() {
                    "FLOW" => matches!(
                        assertion.field.as_str(),
                        "kind" | "text" | "condition" | "branchOutcome" | "target"
                    ),
                    "SYMBOL" => matches!(
                        assertion.field.as_str(),
                        "name" | "returnType" | "parameters"
                    ),
                    "HTTP" | "ROUTE" | "SPRING_ROUTE" => {
                        matches!(assertion.field.as_str(), "method" | "path")
                    }
                    "CONTRACT_FIELD" => {
                        matches!(assertion.field.as_str(), "name" | "type" | "required")
                    }
                    _ => false,
                });
            let actual = observed
                .and_then(|d| d.normalized.get(&assertion.field))
                .filter(|v| !v.is_null());
            let status = if !supported || actual.is_none() {
                "UNKNOWN"
            } else if actual == Some(&assertion.expected) {
                "SUPPORTED"
            } else {
                "CONTRADICTED"
            };
            let result = json!({"claim":id,"check":index,"status":status,"predicate":assertion,"actual":actual,"authority":observed.map(|d|d.normalized.get("authority").cloned().unwrap_or(json!("PROVIDER_EVIDENCE")))});
            if status == "CONTRADICTED" || (status == "UNKNOWN" && claim.uncertainty.is_none()) {
                self.diagnostics.push(json!({"code":if status=="CONTRADICTED"{"CLAIM_CONTRADICTED"}else{"UNSUPPORTED_CLAIM_NEEDS_GAP"},"claim":id,"slot":slot,"check":index,"expected":assertion.expected,"actual":actual,"nextAction":if status=="CONTRADICTED"{"Correct the claim against the supplied provider fact."}else{"Record a specific uncertainty or obtain evidence from a supporting provider; a model cannot supply missing authority."}}));
            }
            results.push(result);
        }
        let fragment = Fragment {
            id: id.clone(),
            text: claim.text.clone(),
            dependency_ids: deps,
            source_ids: sources,
        };
        self.claims.insert(id,json!({"version":digest(&(&fragment,&claim.checks,&claim.uncertainty))?,"slot":slot,"fragment":fragment,"checks":results,"meaning":"UNASSESSED","uncertainty":claim.uncertainty,"influence":bindings::expand_dependencies(&fragment.dependency_ids,&self.work.checked)?}));
        Ok(fragment)
    }
    fn steps(
        &mut self,
        scope: &str,
        steps: &[Step],
        prefix: &str,
        depth: usize,
        op: &mut Operation,
        actors: &BTreeMap<String, String>,
    ) -> Result<(), ClewError> {
        if depth > 16 {
            return Err(invalid("proposal diagram exceeds nesting depth 16"));
        }
        for (index, step) in steps.iter().enumerate() {
            self.events += 1;
            if self.events > 256 {
                return Err(invalid("proposal exceeds 256 authored steps"));
            }
            let slot = format!("{prefix}/{index}");
            let fragment = self.claim(scope, &slot, &step.meaning)?;
            let grouped = matches!(step.kind.as_str(), "alt" | "loop" | "opt");
            if !matches!(
                step.kind.as_str(),
                "message" | "return" | "note" | "alt" | "loop" | "opt" | "declared"
            ) || (!grouped && !step.children.is_empty())
                || (step.kind != "alt" && !step.otherwise.is_empty())
            {
                return Err(invalid(format!(
                    "{slot}: invalid diagram meaning or branch children"
                )));
            }
            let actor = |value: &Option<String>| -> Result<Option<String>, ClewError> {
                value
                    .as_ref()
                    .map(|a| {
                        actors
                            .get(a)
                            .cloned()
                            .ok_or_else(|| invalid(format!("unknown work participant {a}")))
                    })
                    .transpose()
            };
            let interaction = step
                .interaction
                .as_ref()
                .map(|reference| {
                    let h = self.handle(reference)?;
                    self.work
                        .checked
                        .dependencies
                        .get(&h.id)
                        .filter(|d| d.kind == "DECLARED_INTERACTION")
                        .map(|d| d.symbol.clone())
                        .ok_or_else(|| {
                            invalid(
                                "interaction requires a declared-interaction evidence reference",
                            )
                        })
                })
                .transpose()?;
            let event = Event {
                id: fragment.id.clone(),
                kind: step.kind.clone(),
                text: fragment.text.clone(),
                from: actor(&step.from)?,
                to: actor(&step.to)?,
                dependency_ids: fragment.dependency_ids.clone(),
                source_ids: fragment.source_ids.clone(),
                interaction,
            };
            op.events.push(event.clone());
            op.explanation.push(Explanation {
                id: stable(scope, &format!("paragraph/{slot}"))?,
                text: fragment.text.clone(),
                event_ids: vec![fragment.id],
                dependency_ids: fragment.dependency_ids,
                source_ids: fragment.source_ids,
                detail: false,
            });
            if let Some(uncertainty) = &step.meaning.uncertainty {
                op.boundaries.push(uncertainty.clone());
            }
            if grouped {
                self.steps(
                    scope,
                    &step.children,
                    &format!("{slot}/children"),
                    depth + 1,
                    op,
                    actors,
                )?;
                if !step.otherwise.is_empty() {
                    let mut other = event.clone();
                    other.id = stable(scope, &format!("{slot}/else"))?;
                    other.kind = "else".into();
                    other.text = "Otherwise".into();
                    op.explanation.push(Explanation {
                        id: stable(scope, &format!("paragraph/{slot}/else"))?,
                        text: "The source describes an alternative branch.".into(),
                        event_ids: vec![other.id.clone()],
                        dependency_ids: other.dependency_ids.clone(),
                        source_ids: other.source_ids.clone(),
                        detail: false,
                    });
                    op.events.push(other);
                    self.steps(
                        scope,
                        &step.otherwise,
                        &format!("{slot}/otherwise"),
                        depth + 1,
                        op,
                        actors,
                    )?;
                }
                let mut end = event;
                end.id = stable(scope, &format!("{slot}/end"))?;
                end.kind = "end".into();
                end.text = String::new();
                op.events.push(end);
            }
        }
        Ok(())
    }
}

type MaterializedProposal = (Narrative, BTreeMap<String, Value>, Vec<Value>);

fn validate_specialized_fields(
    work: &Work,
    root: &str,
    proposed: &ProposedOperation,
) -> Result<(), ClewError> {
    if proposed.participants.len() > 22 {
        return Err(invalid("proposal exceeds 22 additional participants"));
    }
    if proposed.explanation.len() > 64 {
        return Err(invalid("proposal exceeds 64 explanation paragraphs"));
    }
    if proposed.participants.iter().any(|participant| {
        !store::valid_id(&participant.id)
            || participant.label.trim().is_empty()
            || participant.label.len() > 512
            || participant
                .service
                .as_ref()
                .is_some_and(|service| !store::valid_id(service))
    }) {
        return Err(invalid("invalid proposal participant identity or label"));
    }

    let has_sequence_only_fields =
        !proposed.participants.is_empty() || !proposed.explanation.is_empty();
    if has_sequence_only_fields && super::dataflow::is_root(&work.checked, &work.subject, root) {
        return Err(invalid(
            "data-flow views do not accept sequence participants or explanation paragraphs",
        ));
    }
    if has_sequence_only_fields
        && (super::sections::contains(root)
            || super::notes::is_root(root)
            || super::processes::overview(&work.checked, &work.subject, root))
    {
        return Err(invalid(
            "summary-only roots do not accept sequence participants or explanation paragraphs",
        ));
    }
    Ok(())
}

fn materialize(
    work: &Work,
    input: &Proposal,
    state: &work::ReadState,
) -> Result<MaterializedProposal, ClewError> {
    if input.schema != "codeclew-documentation-proposal/1.0"
        || input.operations.len() > 100
        || input.gaps.len() > 1024
        || input.uncertainties.len() > 64
        || input
            .uncertainties
            .iter()
            .any(|s| s.trim().is_empty() || s.len() > 2048)
    {
        return Err(invalid("invalid proposal schema or content budget"));
    }
    let mut received: BTreeSet<_> = state
        .receipts
        .values()
        .flat_map(|r| r.supplied.iter().cloned())
        .collect();
    received.extend(work::completed_source_references(work, state)?);
    let mut builder = Builder {
        work,
        received,
        claims: BTreeMap::new(),
        diagnostics: Vec::new(),
        events: 0,
    };
    let mut n = Narrative {
        schema: "codeclew-documentation-narrative/1.3".into(),
        subject: work.subject.clone(),
        context_digest: work.checked.context_digest.clone(),
        operations: Vec::new(),
        gaps: BTreeMap::new(),
    };
    let mut actors = BTreeMap::from([("caller".to_owned(), "caller".to_owned())]);
    let mut participants = vec![Participant {
        id: "caller".into(),
        label: "Caller".into(),
        service: None,
    }];
    let allowed: BTreeSet<_> = if let Some(service) = work.subject.strip_prefix("service:") {
        BTreeSet::from([service.to_owned()])
    } else {
        work.checked
            .scenarios
            .get(&work.subject[9..])
            .map(|s| s.steps.iter().map(|s| s.service.clone()).collect())
            .unwrap_or_default()
    };
    for service in work
        .checked
        .services
        .keys()
        .filter(|id| allowed.contains(*id))
    {
        let id = format!("service-{service}");
        actors.insert(service.clone(), id.clone());
        participants.push(Participant {
            id,
            label: service.clone(),
            service: Some(service.clone()),
        });
    }
    let operation_id = |builder: &Builder, reference: &str| -> Result<String, ClewError> {
        if work.subject.starts_with("scenario:") && reference == work.subject {
            return Ok(work
                .request
                .entrypoint
                .clone()
                .unwrap_or_else(|| work.subject[9..].into()));
        }
        let handle = builder.handle(reference)?;
        if !matches!(
            handle.kind.as_str(),
            "ENTRYPOINT" | "SECTION" | "NOTE" | "PROCESS_ROOT"
        ) {
            return Err(invalid("operation requires an authorable work reference"));
        }
        if !operation_reference_allowed(work, handle) {
            return Err(invalid("operation is outside the requested entrypoint"));
        }
        Ok(if handle.kind == "NOTE" {
            super::notes::root(
                handle
                    .id
                    .strip_prefix("note:")
                    .ok_or_else(|| invalid("invalid note work reference"))?,
            )
        } else {
            handle.id.clone()
        })
    };
    for (index, proposed) in input.operations.iter().enumerate() {
        if proposed.title.len() > 512 {
            return Err(invalid("operation title exceeds 512 bytes"));
        }
        let id = operation_id(&builder, &proposed.entrypoint)?;
        validate_specialized_fields(work, &id, proposed)?;
        if proposed.visuals.is_none()
            && work.retained.as_ref().is_some_and(|retained| {
                retained
                    .operations
                    .iter()
                    .any(|operation| operation.id == id && !operation.visuals.is_empty())
            })
        {
            return Err(invalid(
                "existing visual artifacts require an explicit visuals array: include the reviewed replacement set, or [] to remove them; a summary-only update cannot silently erase diagrams",
            ));
        }
        let scope = format!("{}/{}", work.subject, id);
        let summary = builder.claim(&scope, "summary", &proposed.summary)?;
        let mut op = Operation {
            documentation_language: Some(work.request.documentation_language().into()),
            visuals: Vec::new(),
            id,
            title: proposed.title.clone(),
            summary,
            assessment: None,
            dataflow: None,
            explanation: Vec::new(),
            interface_contracts: Vec::new(),
            overview_diagram: None,
            participants: participants.clone(),
            events: Vec::new(),
            findings: Vec::new(),
            boundaries: input.uncertainties.clone(),
        };
        if let Some(gap) = &proposed.summary.uncertainty {
            op.boundaries.push(gap.clone());
        }
        let proposed_visuals = proposed.visuals.as_deref().unwrap_or_default();
        op.visuals = super::visuals::materialize(proposed_visuals, |slot, claim| {
            builder.claim(&scope, slot, claim)
        })?;
        if super::dataflow::is_root(&work.checked, &work.subject, &op.id) {
            let graph = proposed
                .dataflow
                .as_ref()
                .ok_or_else(|| invalid("data-flow view requires typed nodes and edges"))?;
            if !proposed.steps.is_empty()
                || !proposed.contracts.is_empty()
                || proposed.assessment.is_some()
                || !proposed_visuals.is_empty()
                || !proposed.participants.is_empty()
                || !proposed.explanation.is_empty()
            {
                return Err(invalid(
                    "data-flow graphs are separate from sequence steps, contracts, participants, explanation paragraphs and note assessments",
                ));
            }
            op.dataflow = Some(super::dataflow::materialize(
                &work.checked,
                &work.subject[9..],
                graph,
                |slot, claim| builder.claim(&scope, slot, claim),
            )?);
            op.boundaries.extend(
                work.checked.scenarios[&work.subject[9..]]
                    .boundaries
                    .clone(),
            );
            op.participants.clear();
            n.operations.push(op);
            continue;
        } else if proposed.dataflow.is_some() {
            return Err(invalid(
                "typed data-flow content requires a saved view root",
            ));
        }
        if super::notes::is_root(&op.id) && !op.visuals.is_empty() {
            return Err(invalid("note assessments cannot contain visual artifacts"));
        }
        if super::notes::is_root(&op.id) {
            let a = proposed
                .assessment
                .as_ref()
                .ok_or_else(|| invalid("note assessment requires an outcome and period"))?;
            let note_id = op.id.strip_prefix("assessment-").unwrap();
            let note = work
                .checked
                .dependencies
                .get(&format!("note:{note_id}"))
                .ok_or_else(|| invalid("unknown note"))?;
            builder.handle(&proposed.entrypoint)?;
            if !op.summary.dependency_ids.contains(&note.id) {
                return Err(invalid(
                    "assessment must cite its captured note as well as supporting code",
                ));
            }
            op.assessment = Some(super::notes::Assessment {
                schema: "codeclew-documentation-note-assessment/1.0".into(),
                note: note_id.into(),
                note_digest: note.normalized["original"]["digest"]
                    .as_str()
                    .unwrap_or("")
                    .into(),
                association_digest: note.normalized["associationDigest"]
                    .as_str()
                    .unwrap_or("")
                    .into(),
                outcome: a.outcome.clone(),
                period: a.period.clone(),
                proposed_correction: a
                    .proposed_correction
                    .as_ref()
                    .map(|c| builder.claim(&scope, "correction", c))
                    .transpose()?,
            });
        } else if proposed.assessment.is_some() {
            return Err(invalid("assessments require an explicit note root"));
        }
        if super::sections::contains(&op.id)
            || super::notes::is_root(&op.id)
            || super::processes::overview(&work.checked, &work.subject, &op.id)
        {
            if !proposed.steps.is_empty()
                || !proposed.contracts.is_empty()
                || !proposed.participants.is_empty()
                || !proposed.explanation.is_empty()
            {
                return Err(invalid(
                    "summary-only proposals use a supported summary; operation sequences, participants and explanation paragraphs are separate",
                ));
            }
            if super::processes::overview(&work.checked, &work.subject, &op.id) {
                op.boundaries.extend(
                    work.checked.scenarios[&work.subject[9..]]
                        .boundaries
                        .clone(),
                );
            }
            op.participants.clear();
            n.operations.push(op);
            continue;
        }
        let mut op_actors = actors.clone();
        for declared in &proposed.participants {
            if !store::valid_id(&declared.id)
                || op_actors
                    .insert(declared.id.clone(), declared.id.clone())
                    .is_some()
            {
                return Err(invalid(format!(
                    "invalid or duplicate declared participant {}",
                    declared.id
                )));
            }
            op.participants.push(Participant {
                id: declared.id.clone(),
                label: declared.label.clone(),
                service: declared.service.clone(),
            });
        }
        if op.participants.len() > 24 {
            return Err(invalid("sequence exceeds 24 participants"));
        }
        builder.steps(&scope, &proposed.steps, "step", 0, &mut op, &op_actors)?;
        if !proposed.explanation.is_empty() {
            // Authored narrative is added to the per-step echo so arrows can stay
            // short while "What happens" carries the domain prose. Anchor each
            // paragraph to the first non-end event and fold in its evidence so
            // required step coverage and evidence retention hold.
            let anchor = op.events.iter().find(|e| e.kind != "end").cloned();
            for (i, claim) in proposed.explanation.iter().enumerate() {
                let mut fragment = builder.claim(&scope, &format!("narrative/{i}"), claim)?;
                let mut event_ids = Vec::new();
                if let Some(event) = &anchor {
                    event_ids.push(event.id.clone());
                    fragment
                        .dependency_ids
                        .extend(event.dependency_ids.iter().cloned());
                    fragment.source_ids.extend(event.source_ids.iter().cloned());
                }
                op.explanation.push(Explanation {
                    id: stable(&scope, &format!("paragraph/narrative/{i}"))?,
                    text: fragment.text,
                    event_ids,
                    dependency_ids: fragment.dependency_ids,
                    source_ids: fragment.source_ids,
                    detail: false,
                });
            }
        }
        if proposed.contracts.len() > 64 {
            return Err(invalid("proposal exceeds 64 contracts"));
        }
        for (ci, contract) in proposed.contracts.iter().enumerate() {
            let mut rows = Vec::new();
            if contract.rows.len() > 128 {
                return Err(invalid("contract exceeds 128 rows"));
            }
            for (ri, row) in contract.rows.iter().enumerate() {
                let fragment =
                    builder.claim(&scope, &format!("contract/{ci}/{ri}"), &row.description)?;
                rows.push(InterfaceContractRow {
                    id: fragment.id,
                    label: row.label.clone(),
                    value: fragment.text,
                    dependency_ids: fragment.dependency_ids,
                    source_ids: fragment.source_ids,
                });
            }
            op.interface_contracts.push(InterfaceContract {
                id: stable(&scope, &format!("contract/{ci}"))?,
                title: contract.title.clone(),
                kind: contract.kind.clone(),
                rows,
                boundaries: contract
                    .rows
                    .iter()
                    .filter_map(|r| r.description.uncertainty.clone())
                    .collect(),
            });
        }
        // A sequence is not an overview topology. Keep its branch and return events
        // intact instead of manufacturing disconnected overview nodes that hide it.
        op.boundaries.extend(
            work.obligations
                .iter()
                .filter_map(|o| o["detail"].as_str().map(str::to_owned)),
        );
        n.operations.push(op);
        if let Err(error) = render::validate(
            &Narrative {
                gaps: expected(work)
                    .into_iter()
                    .filter(|id| !n.operations.iter().any(|o| &o.id == id))
                    .map(|id| (id, "Not included in this operation validation.".into()))
                    .collect(),
                ..n.clone()
            },
            &work.checked,
        ) {
            builder.diagnostics.push(json!({"code":"STRUCTURE_OR_COVERAGE_INVALID","operation":index,"nextAction":error.message}));
        }
    }
    for (reference, reason) in &input.gaps {
        let id = if let Some(h) = work.handles.get(reference) {
            if !gap_reference_allowed(h) {
                return Err(invalid(
                    "gap requires an entrypoint reference or its scenario subject",
                ));
            }
            if h.kind == "NOTE" {
                super::notes::root(
                    h.id.strip_prefix("note:")
                        .ok_or_else(|| invalid("invalid note work reference"))?,
                )
            } else {
                h.id.clone()
            }
        } else if work.subject.starts_with("scenario:") && reference == &work.subject {
            work.request
                .entrypoint
                .clone()
                .unwrap_or_else(|| work.subject[9..].into())
        } else {
            return Err(invalid(
                "gap requires an entrypoint reference or its scenario subject",
            ));
        };
        n.gaps.insert(id, reason.clone());
    }
    if work.subject.starts_with("service:") {
        for id in super::sections::ids().filter(|id| !n.operations.iter().any(|o| &o.id == id)) {
            n.gaps.entry(id).or_insert_with(|| {
                "Required service section has not been authored in this proposal.".into()
            });
        }
    }
    if let Some(entrypoint) = &work.request.entrypoint {
        for id in expected(work).into_iter().filter(|id| id != entrypoint) {
            n.gaps
                .entry(id)
                .or_insert_with(|| "Outside the requested work entrypoint.".into());
        }
    }
    if let Err(error) = render::validate(&n, &work.checked) {
        builder
            .diagnostics
            .push(json!({"code":"STRUCTURE_OR_COVERAGE_INVALID","nextAction":error.message}));
    }
    Ok((n, builder.claims, builder.diagnostics))
}
fn expected(work: &Work) -> BTreeSet<String> {
    if let Some(service) = work.subject.strip_prefix("service:") {
        super::notes::expected(&work.checked, service)
    } else {
        super::processes::expected(&work.checked, &work.subject[9..])
    }
}

pub fn submit(repo: &Repository, id: &str, input: Proposal) -> Result<Value, ClewError> {
    let work = work::load(repo, id)?;
    current(repo, &work)?;
    let state = work::read_state(repo, id)?;
    let read_digest = digest(&state)?;
    let (narrative, claims, mut diagnostics) = match materialize(&work, &input, &state) {
        Ok((n, c, d)) => (Some(n), c, d),
        Err(error) => (
            None,
            BTreeMap::new(),
            vec![json!({"code":"INVALID_PROPOSAL","nextAction":error.message})],
        ),
    };
    if !work::initial_context_complete_with_parts(&work, &state)? {
        diagnostics.push(json!({"code":"REQUIRED_CONTEXT_NOT_READ","nextAction":"Read every initial Work page. For an oversized required SOURCE, use docs work read-part with its SOURCE reference and continue until nextCursor is null; automatic authoring still requires the complete context to fit in its actual request."}));
    }
    for obligation in work.obligations.iter().filter(|o| {
        matches!(
            o["kind"].as_str(),
            Some("MISSING_SERVICE" | "MISSING_EXTERNAL_INPUT")
        )
    }) {
        diagnostics.push(json!({"code":"MISSING_WORK_EVIDENCE","obligation":obligation,"nextAction":"Restore or register the required input and prepare new work before author review."}));
    }
    if state.untracked_reads {
        diagnostics.push(json!({"code":"INCOMPLETE_INFLUENCE","nextAction":"Prepare a new work with every required input registered and use recorded reads only."}));
    }
    let status = if !diagnostics.is_empty() {
        "NEEDS_REPAIR"
    } else if !input.uncertainties.is_empty()
        || !input.gaps.is_empty()
        || !work.obligations.is_empty()
        || input.operations.iter().any(|operation| {
            operation
                .visuals
                .iter()
                .flatten()
                .any(|visual| !visual.limitations.is_empty())
        })
        || claims.values().any(|c| !c["uncertainty"].is_null())
    {
        "READY_WITH_LIMITATIONS"
    } else {
        "READY_FOR_REVIEW"
    };
    let artifact = Artifact {
        schema: "codeclew-documentation-proposal-result/1.0".into(),
        id: String::new(),
        work: id.into(),
        input,
        narrative,
        status: status.into(),
        diagnostics,
        claims,
        read_digest,
        influence: work.influence.clone(),
        meaning_review: "UNASSESSED".into(),
    };
    let stored = StoredArtifact::from_runtime(artifact)?;
    let artifact = &stored.artifact;
    let encoded = bytes(&stored)?;
    if encoded.len() > 4 * 1024 * 1024 {
        return Err(invalid("materialized proposal exceeds 4 MiB"));
    }
    {
        let _lock = repo.lock()?;
        if digest(&work::read_state(repo, id)?)? != artifact.read_digest {
            return Err(invalid(
                "work reads changed during proposal submission; retry submission",
            ));
        }
        let path = file(&artifact.id)?;
        if repo.path(&path)?.exists() {
            load(repo, &artifact.id)?;
        } else {
            repo.atomic(&path, &encoded)?;
        }
    }
    show(repo, &artifact.id, None, 20)
}
pub fn show(
    repo: &Repository,
    id: &str,
    cursor: Option<&str>,
    limit: usize,
) -> Result<Value, ClewError> {
    let a = load(repo, id)?;
    let mut rows = Vec::new();
    for (index, d) in a.diagnostics.iter().enumerate() {
        rows.push(json!({"kind":"DIAGNOSTIC","id":format!("diagnostic-{index}"),"record":d}));
    }
    for (id, claim) in &a.claims {
        rows.push(json!({"kind":"CLAIM","id":id,"record":claim}));
    }
    if let Some(n) = &a.narrative {
        for operation in &n.operations {
            rows.push(json!({"kind":"OPERATION","id":operation.id,"record":{"title":operation.title,"summary":operation.summary,"participants":operation.participants,"boundaries":operation.boundaries,"overviewDiagram":operation.overview_diagram}}));
            for visual in &operation.visuals {
                rows.push(json!({"kind":"VISUAL","id":format!("{}/{}",operation.id,visual.id),"record":visual}));
            }
            if let Some(g) = &operation.dataflow {
                rows.push(json!({"kind":"DATAFLOW_BINDING","id":operation.id,"record":{"schema":g.schema,"view":g.view,"definitionDigest":g.definition_digest,"moduleDigest":g.module_digest}}));
                for node in &g.nodes {
                    rows.push(json!({"kind":"DATAFLOW_NODE","id":node.id,"record":node}));
                }
                for edge in &g.edges {
                    rows.push(json!({"kind":"DATAFLOW_EDGE","id":edge.id,"record":edge}));
                }
            }
            for event in &operation.events {
                rows.push(json!({"kind":"DIAGRAM_STEP","id":event.id,"record":event}));
            }
            for paragraph in &operation.explanation {
                rows.push(json!({"kind":"EXPLANATION","id":paragraph.id,"record":paragraph}));
            }
            for contract in &operation.interface_contracts {
                rows.push(json!({"kind":"CONTRACT","id":contract.id,"record":{"title":contract.title,"kind":contract.kind,"boundaries":contract.boundaries}}));
                for row in &contract.rows {
                    rows.push(json!({"kind":"CONTRACT_ROW","id":row.id,"record":row}));
                }
            }
        }
        for (id, gap) in &n.gaps {
            rows.push(json!({"kind":"GAP","id":id,"record":gap}));
        }
    }
    super::cli::page(
        id,
        rows,
        cursor,
        limit,
        json!({"reportSchema":a.schema,"proposal":id,"work":a.work,"status":a.status,"meaningReview":a.meaning_review,"readDigest":a.read_digest,"narrativeDigest":a.narrative.as_ref().map(digest).transpose()?}),
    )
}

#[cfg(test)]
mod storage_tests {
    use super::*;

    fn artifact(influence: BTreeMap<String, String>) -> Artifact {
        Artifact {
            schema: "codeclew-documentation-proposal-result/1.0".into(),
            id: String::new(),
            work: "a".repeat(64),
            input: Proposal {
                schema: "codeclew-documentation-proposal/1.0".into(),
                operations: vec![],
                gaps: BTreeMap::new(),
                uncertainties: vec![],
            },
            narrative: None,
            status: "NEEDS_REPAIR".into(),
            diagnostics: vec![],
            claims: BTreeMap::new(),
            read_digest: "sha256:reads".into(),
            influence,
            meaning_review: "UNASSESSED".into(),
        }
    }

    #[test]
    fn large_influence_is_shared_without_changing_its_authority() {
        let influence: BTreeMap<_, _> = (0..40_000)
            .map(|i| {
                (
                    format!("service:dependency:{i:08}:{}", "x".repeat(40)),
                    format!("sha256:{}", "b".repeat(64)),
                )
            })
            .collect();
        assert!(bytes(&influence).unwrap().len() > 4 * 1024 * 1024);
        let original = artifact(influence.clone());
        let stored = StoredArtifact::from_runtime(original).unwrap();
        let encoded = bytes(&stored).unwrap();
        assert!(encoded.len() < 2048);
        let decoded: StoredArtifact = serde_json::from_slice(&encoded).unwrap();
        decoded.validate_identity(&stored.artifact.id).unwrap();
        let hydrated = decoded.hydrate(influence.clone()).unwrap();
        assert_eq!(hydrated.influence, influence);
        assert_eq!(hydrated.work, "a".repeat(64));
        assert_eq!(hydrated.id, stored.artifact.id);
        assert!(stored.hydrate(BTreeMap::new()).is_err());
    }

    #[test]
    fn proposal_record_rejects_mutation_and_competing_inline_authority() {
        let stored = StoredArtifact::from_runtime(artifact(BTreeMap::new())).unwrap();
        let id = stored.artifact.id.clone();
        let mut changed = stored.clone();
        changed.artifact.work = "b".repeat(64);
        assert!(changed.validate_identity(&id).is_err());
        let mut changed = stored.clone();
        changed.influence_digest = "sha256:wrong".into();
        assert!(changed.validate_identity(&id).is_err());
        let mut changed = stored;
        changed
            .artifact
            .influence
            .insert("unexpected".into(), "digest".into());
        changed.artifact.id.clear();
        changed.artifact.id = digest(&changed).unwrap()[7..].into();
        assert!(changed.validate_identity(&changed.artifact.id).is_err());
        assert!(
            serde_json::from_slice::<StoredArtifact>(&bytes(&artifact(BTreeMap::new())).unwrap())
                .is_err()
        );
    }
}

#[cfg(test)]
mod operation_input_tests {
    use super::*;
    use serde_json::json;

    fn work(root: &str) -> Work {
        let subject = "scenario:dispatch".to_string();
        let mut checked = check::assemble(
            "input".into(),
            BTreeMap::new(),
            BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
        .unwrap();
        let (id, kind) = if root == super::super::processes::OVERVIEW {
            ("process:dispatch", "PROCESS_DEFINITION")
        } else {
            ("view:dispatch", "VIEW_DEFINITION")
        };
        checked.dependencies.insert(
            id.into(),
            super::super::model::Observation {
                id: id.into(),
                kind: kind.into(),
                service: String::new(),
                symbol: id.into(),
                normalized: json!({}),
                digest: "digest".into(),
                source_ids: Vec::new(),
            },
        );
        Work {
            schema: "codeclew-documentation-work/1.0".into(),
            id: "work".into(),
            subject: subject.clone(),
            request: work::Request {
                schema: "codeclew-documentation-work-request/1.0".into(),
                audience: "Maintainers".into(),
                documentation_language: None,
                entrypoint: Some(root.into()),
                context_profile: None,
                max_items: 20,
                max_bytes: 40960,
                external_inputs: Vec::new(),
            },
            checked,
            snapshot: Some("snapshot".into()),
            retained: None,
            external_inputs: BTreeMap::new(),
            handles: BTreeMap::new(),
            influence: BTreeMap::new(),
            obligations: Vec::new(),
            review_reasons: Vec::new(),
        }
    }

    fn proposal(participants: Vec<ParticipantInput>, explanation: Vec<Claim>) -> Proposal {
        Proposal {
            schema: "codeclew-documentation-proposal/1.0".into(),
            operations: vec![ProposedOperation {
                entrypoint: "scenario:dispatch".into(),
                title: "Dispatch".into(),
                summary: Claim {
                    text: String::new(),
                    evidence: Vec::new(),
                    checks: Vec::new(),
                    uncertainty: None,
                },
                assessment: None,
                dataflow: None,
                steps: Vec::new(),
                contracts: Vec::new(),
                participants,
                explanation,
                visuals: None,
            }],
            gaps: BTreeMap::new(),
            uncertainties: Vec::new(),
        }
    }

    #[test]
    fn summary_and_dataflow_roots_reject_sequence_only_fields() {
        let participant = || {
            vec![ParticipantInput {
                id: "worker".into(),
                label: "Worker".into(),
                service: None,
            }]
        };
        let explanation = || {
            vec![Claim {
                text: "The worker handles the request.".into(),
                evidence: vec!["source-1".into()],
                checks: Vec::new(),
                uncertainty: None,
            }]
        };
        for (root, expected) in [
            (super::super::processes::OVERVIEW, "summary-only"),
            (super::super::dataflow::ROOT, "data-flow"),
        ] {
            for proposal in [
                proposal(participant(), Vec::new()),
                proposal(Vec::new(), explanation()),
            ] {
                let error =
                    materialize(&work(root), &proposal, &work::ReadState::default()).unwrap_err();
                assert!(error.message.contains(expected), "{}", error.message);
            }
        }
    }
}
