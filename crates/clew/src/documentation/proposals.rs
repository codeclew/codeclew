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
    pub entrypoint: String,
    pub title: String,
    pub summary: Claim,
    pub steps: Vec<Step>,
    #[serde(default)]
    pub contracts: Vec<Contract>,
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
    let mut result: Artifact = store::read(&repo.path(&file(id)?)?, 4 * 1024 * 1024)?;
    let recorded = result.id.clone();
    result.id.clear();
    let expected = digest(&result)?[7..].to_owned();
    result.id = recorded;
    if result.id != id
        || expected != id
        || result.schema != "codeclew-documentation-proposal-result/1.0"
    {
        return Err(invalid("proposal digest or schema is invalid"));
    }
    Ok(result)
}
pub fn run(command: Command) -> Result<Value, ClewError> {
    match command {
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

/// Recheck exact revisions and every admitted external input before acceptance.
pub fn current(repo: &Repository, work: &Work) -> Result<(), ClewError> {
    let (kind, id) = work
        .subject
        .split_once(':')
        .ok_or_else(|| invalid("invalid work subject"))?;
    let selected = if kind == "service" {
        BTreeSet::from([id.to_owned()])
    } else {
        BTreeSet::new()
    };
    let now = check::run_selected(repo, &selected)?;
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
    if retained != work.retained {
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
impl Builder<'_> {
    fn handle(&self, reference: &str) -> Result<&work::Handle, ClewError> {
        if !self.received.contains(reference) {
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
            match handle.kind.as_str() {
                "DEPENDENCY" => {
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
                            .filter(|d| d.source_ids.contains(&handle.id))
                            .map(|d| d.id.clone()),
                    );
                }
                _ => return Err(invalid("unsupported evidence reference")),
            }
        }
        if deps.len() > 128 || sources.is_empty() || sources.len() > 32 {
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

fn materialize(
    work: &Work,
    input: &Proposal,
    state: &work::ReadState,
) -> Result<(Narrative, BTreeMap<String, Value>, Vec<Value>), ClewError> {
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
    let received = state
        .receipts
        .values()
        .flat_map(|r| r.supplied.iter().cloned())
        .collect();
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
            return Ok(work.subject[9..].into());
        }
        let handle = builder.handle(reference)?;
        if handle.kind != "ENTRYPOINT" {
            return Err(invalid("operation requires an entrypoint work reference"));
        }
        if work
            .request
            .entrypoint
            .as_ref()
            .is_some_and(|id| id != &handle.id)
        {
            return Err(invalid("operation is outside the requested entrypoint"));
        }
        Ok(handle.id.clone())
    };
    for (index, proposed) in input.operations.iter().enumerate() {
        if proposed.title.len() > 512 {
            return Err(invalid("operation title exceeds 512 bytes"));
        }
        let id = operation_id(&builder, &proposed.entrypoint)?;
        let scope = format!("{}/{}", work.subject, id);
        let summary = builder.claim(&scope, "summary", &proposed.summary)?;
        let mut op = Operation {
            id,
            title: proposed.title.clone(),
            summary,
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
        builder.steps(&scope, &proposed.steps, "step", 0, &mut op, &actors)?;
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
        let nodes: Vec<_> = op
            .events
            .iter()
            .filter(|e| e.kind != "end" && e.kind != "else")
            .take(12)
            .enumerate()
            .map(|(i, e)| DiagramNode {
                id: format!("node-{i}"),
                text: e.text.chars().take(84).collect(),
                participant: e
                    .to
                    .clone()
                    .or_else(|| e.from.clone())
                    .unwrap_or_else(|| participants[1].id.clone()),
                column: (i % 4) as u8,
                row: (i / 4) as u8,
                event_ids: vec![e.id.clone()],
            })
            .collect();
        op.overview_diagram = Some(OverviewDiagram {
            nodes,
            edges: Vec::new(),
        });
        if op.events.iter().filter(|e| e.kind != "end").count() > 12 {
            op.boundaries.push("The overview shows the first twelve steps; the complete sequence and explanation retain every step.".into());
        }
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
        let id = if let Some(h) = work
            .handles
            .get(reference)
            .filter(|h| h.kind == "ENTRYPOINT")
        {
            h.id.clone()
        } else if work.subject.starts_with("scenario:") && reference == &work.subject {
            work.subject[9..].into()
        } else {
            return Err(invalid(
                "gap requires an entrypoint reference or its scenario subject",
            ));
        };
        n.gaps.insert(id, reason.clone());
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
        work.checked
            .services
            .get(service)
            .map(|s| s.entrypoints.iter().map(|e| e.id.clone()).collect())
            .unwrap_or_default()
    } else {
        BTreeSet::from([work.subject[9..].into()])
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
    if !work::initial_context_complete(&state) {
        diagnostics.push(json!({"code":"REQUIRED_CONTEXT_NOT_READ","nextAction":"Read every initial work page; resolve oversized required items by preparing an adequate evidence budget."}));
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
        || claims.values().any(|c| !c["uncertainty"].is_null())
    {
        "READY_WITH_LIMITATIONS"
    } else {
        "READY_FOR_REVIEW"
    };
    let mut artifact = Artifact {
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
    artifact.id = digest(&artifact)?[7..].into();
    let encoded = bytes(&artifact)?;
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
