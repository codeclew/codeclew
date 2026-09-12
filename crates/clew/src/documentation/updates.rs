//! Coordinator-selected revisions are independent of producer arrival order.
use super::{
    bindings, bytes, digest, invalid, io_error,
    model::*,
    store::{self, Repository},
};
use crate::error::ClewError;
use clap::Subcommand;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

const STATE: &str = "catalog/update-state.json";
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Policy {
    pub schema: String,
    pub service: String,
    pub repository_id: String,
    pub accepted_refs: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Event {
    pub schema: String,
    pub id: String,
    pub service: String,
    pub repository_id: String,
    pub source_ref: String,
    pub revision: String,
    pub sequence: u64,
    #[serde(default)]
    pub tag: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct State {
    pub schema: String,
    pub targets: BTreeMap<String, Event>,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RevisionSet {
    schema: String,
    events: Vec<Event>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    Configure {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        expected_input_digest: String,
    },
    Enqueue {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        input: PathBuf,
    },
    Reconcile {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        input: PathBuf,
    },
    Run {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        input: Option<PathBuf>,
        #[arg(long)]
        config: Option<PathBuf>,
        #[arg(long,default_value_t=8,value_parser=clap::value_parser!(u32).range(1..=64))]
        max_work: u32,
    },
    Status {
        #[arg(long)]
        root: PathBuf,
    },
}
fn revision(s: &str) -> bool {
    s.len() == 40
        && s.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
fn reference(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 256
        && !s.starts_with('-')
        && !s.chars().any(|c| c.is_whitespace() || c.is_control())
}
pub(super) fn policies(repo: &Repository) -> Result<BTreeMap<String, Policy>, ClewError> {
    let rows: BTreeMap<String, Policy> = repo.records("catalog/update-policy", "json")?;
    for (id, p) in &rows {
        if p.schema != "codeclew-documentation-update-policy/1.0"
            || id != &p.service
            || !store::valid_id(&p.repository_id)
            || p.accepted_refs.is_empty()
            || p.accepted_refs.len() > 32
            || !p.accepted_refs.iter().all(|r| reference(r))
            || p.accepted_refs.iter().collect::<BTreeSet<_>>().len() != p.accepted_refs.len()
        {
            return Err(invalid("invalid update ref policy"));
        }
    }
    Ok(rows)
}
pub(super) fn state(repo: &Repository) -> Result<State, ClewError> {
    let path = repo.path(STATE)?;
    if !path.exists() {
        return Ok(State {
            schema: "codeclew-documentation-update-state/1.0".into(),
            targets: BTreeMap::new(),
        });
    }
    let s: State = store::read(&path, store::MAX_RECORD)?;
    if s.schema != "codeclew-documentation-update-state/1.0" || s.targets.len() > 1024 {
        return Err(invalid("invalid coordinator target state"));
    }
    for (id, e) in &s.targets {
        validate(e)?;
        if id != &e.service {
            return Err(invalid("coordinator target service mismatch"));
        }
    }
    Ok(s)
}
fn validate(e: &Event) -> Result<(), ClewError> {
    if e.schema != "codeclew-documentation-update-event/1.0"
        || !store::valid_id(&e.id)
        || !store::valid_id(&e.service)
        || !store::valid_id(&e.repository_id)
        || !reference(&e.source_ref)
        || !revision(&e.revision)
        || e.sequence == 0
        || e.tag
            .as_ref()
            .is_some_and(|t| !reference(t) || t != &e.source_ref)
    {
        return Err(invalid(
            "invalid revision event identity, ref, tag or sequence",
        ));
    }
    Ok(())
}
fn read_events(input: &std::path::Path) -> Result<Vec<Event>, ClewError> {
    let value: Value = store::read(input, store::MAX_RECORD)?;
    if value["schema"] == "codeclew-documentation-revision-set/1.0" {
        let set: RevisionSet = serde_json::from_value(value).map_err(io_error)?;
        if set.schema != "codeclew-documentation-revision-set/1.0"
            || set.events.is_empty()
            || set.events.len() > 64
        {
            return Err(invalid("revision reconciliation requires 1 to 64 events"));
        }
        Ok(set.events)
    } else {
        Ok(vec![serde_json::from_value(value).map_err(io_error)?])
    }
}
fn publish_status(repo: &Repository) -> Result<Value, ClewError> {
    if bindings::baseline(repo)?.is_some() {
        super::status::refresh(repo)
    } else {
        super::render::publish(repo, vec![], false)
    }
}
fn enqueue(repo: &Repository, events: Vec<Event>) -> Result<Value, ClewError> {
    let outcomes;
    {
        let _lock = repo.lock()?;
        let policies = policies(repo)?;
        let services = repo.services()?;
        let mut current = state(repo)?;
        let mut seen = BTreeMap::new();
        let mut rows = Vec::new();
        // Validate the entire reconciliation before advancing any target.
        for e in &events {
            validate(e)?;
            let service = services
                .get(&e.service)
                .ok_or_else(|| invalid("revision event service is unregistered"))?;
            let policy = policies
                .get(&e.service)
                .ok_or_else(|| invalid("configure accepted refs before accepting events"))?;
            if e.repository_id != service.repository_id
                || policy.repository_id != e.repository_id
                || !policy.accepted_refs.contains(&e.source_ref)
            {
                return Err(invalid("event origin or ref is outside coordinator policy"));
            }
            let path = repo.path(&format!("updates/events/{}.json", e.id))?;
            if path.exists() {
                let old: Event = store::read(&path, store::MAX_RECORD)?;
                if &old != e {
                    return Err(invalid(
                        "event idempotency key was reused with different content",
                    ));
                }
            }
            if let Some(old) = seen.insert(&e.id, e)
                && old != e
            {
                return Err(invalid("conflicting event IDs in reconciliation"));
            }
        }
        for e in &events {
            let status = if let Some(old) = current.targets.get(&e.service) {
                if e.sequence < old.sequence {
                    "SUPERSEDED"
                } else if e.sequence == old.sequence {
                    if old != e {
                        return Err(invalid("same service sequence names different events"));
                    }
                    "DUPLICATE"
                } else {
                    "ACCEPTED"
                }
            } else {
                "ACCEPTED"
            };
            if status == "ACCEPTED" {
                current.targets.insert(e.service.clone(), e.clone());
            }
            rows.push(json!({"id":e.id,"service":e.service,"status":status,"revision":e.revision,"sequence":e.sequence}));
        }
        for e in &events {
            repo.atomic(&format!("updates/events/{}.json", e.id), &bytes(e)?)?;
        }
        repo.atomic(STATE, &bytes(&current)?)?;
        outcomes = rows;
    }
    // Target acceptance survives a failed status write; run/reconcile can repair it.
    let publication = match publish_status(repo) {
        Ok(v) => v,
        Err(e) => {
            json!({"status":"STATUS_PUBLICATION_PENDING","reason":e.code,"nextAction":"Retry docs update run after resolving the publication failure."})
        }
    };
    Ok(
        json!({"schema":"codeclew-docs-update-enqueue/1.0","status":"TARGETS_RECONCILED","events":outcomes,"targets":state(repo)?.targets,"publication":publication}),
    )
}

/// Coordinator mode never invokes a service compiler during a central check.
pub(super) fn capture(
    repo: &Repository,
    service: &Service,
    targets: &State,
) -> Result<ServiceEvidence, ClewError> {
    let e = super::evidence_package::selected(repo, service)?
        .ok_or_else(|| invalid("central update requires admitted portable service evidence"))?;
    if let Some(target) = targets.targets.get(&service.id)
        && (target.repository_id != service.repository_id || target.revision != e.revision)
    {
        return Err(invalid(
            "admitted artifact has not reached the coordinator-selected revision",
        ));
    }
    Ok(e)
}
pub(super) fn admit_package(
    repo: &Repository,
    service: &str,
    revision: Option<&str>,
) -> Result<(), ClewError> {
    if let Some(target) = state(repo)?.targets.get(service)
        && revision != Some(target.revision.as_str())
    {
        return Err(invalid(
            "late artifact cannot regress the coordinator-selected target",
        ));
    }
    Ok(())
}
fn run_queue(
    repo: &Repository,
    config: Option<&std::path::Path>,
    max_work: usize,
) -> Result<Value, ClewError> {
    let publication = publish_status(repo)?;
    let target_digest = digest(&state(repo)?)?;
    let checked = super::check::run(repo)?;
    let baseline = bindings::baseline(repo)?
        .ok_or_else(|| invalid("update status publication is unavailable"))?;
    let mut pending = Vec::new();
    for (id, n) in &baseline.1.narratives {
        let expected = if let Some(service) = id.strip_prefix("service:") {
            super::notes::expected(&checked, service)
        } else {
            super::processes::expected(&checked, &id[9..])
        };
        for root in expected {
            let key = format!("{id}/{root}");
            if n.gaps.contains_key(&root)
                || !n.operations.iter().any(|o| o.id == root)
                || baseline.1.section_states.get(&key).is_none_or(|s| {
                    s.freshness != Freshness::Current
                        || !matches!(
                            s.verification.as_str(),
                            "VERIFIED" | "VERIFIED_WITH_LIMITATIONS"
                        )
                })
            {
                pending.push((id.clone(), root));
            }
        }
    }
    let mut results = Vec::new();
    if let Some(config) = config {
        for (subject, root) in pending.iter().take(max_work) {
            if digest(&state(repo)?)? != target_digest {
                results.push(json!({"subject":subject,"root":root,"status":"SUPERSEDED","nextAction":"Run the queue for the newly accepted target vector."}));
                break;
            }
            let entrypoint = if subject.starts_with("service:")
                || root == super::processes::OVERVIEW
                || root == super::dataflow::ROOT
            {
                Some(root.clone())
            } else {
                None
            };
            let request = super::work::Request {
                schema: "codeclew-documentation-work-request/1.0".into(),
                audience: "Maintainers reviewing behavior at the coordinator-selected revision"
                    .into(),
                entrypoint,
                max_items: 20,
                max_bytes: 40 * 1024,
                external_inputs: vec![],
            };
            let result = super::work::prepare(repo, subject.clone(), request).and_then(|w| {
                super::agent_jobs::run(
                    repo,
                    w["work"]
                        .as_str()
                        .ok_or_else(|| invalid("prepared work has no identity"))?,
                    Some(config),
                )
            });
            results.push(match result{Ok(v)=>json!({"subject":subject,"root":root,"result":v}),Err(e)=>json!({"subject":subject,"root":root,"status":"LOCAL_UPDATE_GAP","reason":e.code})});
        }
    }
    Ok(
        json!({"schema":"codeclew-docs-update-run/1.0","status":if config.is_none()&&!pending.is_empty(){"AGENT_CONFIGURATION_REQUIRED"}else{"PROCESSED"},"targetDigest":target_digest,"publication":publication,"pendingCount":pending.len(),"attempted":results.len(),"remainingAtStart":pending.len().saturating_sub(results.len()),"results":results}),
    )
}
pub fn run(command: Command) -> Result<Value, ClewError> {
    match command {
        Command::Configure {
            root,
            input,
            expected_input_digest,
        } => {
            let repo = Repository::open(&root)?;
            let p: Policy = store::read(&input, store::MAX_RECORD)?;
            let service = repo
                .services()?
                .remove(&p.service)
                .ok_or_else(|| invalid("unknown update policy service"))?;
            if p.repository_id != service.repository_id
                || p.schema != "codeclew-documentation-update-policy/1.0"
                || p.accepted_refs.is_empty()
                || p.accepted_refs.len() > 32
                || !p.accepted_refs.iter().all(|r| reference(r))
                || p.accepted_refs.iter().collect::<BTreeSet<_>>().len() != p.accepted_refs.len()
            {
                return Err(invalid("invalid accepted-ref policy"));
            }
            repo.put(
                &format!("catalog/update-policy/{}.json", p.service),
                &p,
                Some(&expected_input_digest),
            )
        }
        Command::Enqueue { root, input } | Command::Reconcile { root, input } => {
            enqueue(&Repository::open(&root)?, read_events(&input)?)
        }
        Command::Run {
            root,
            input,
            config,
            max_work,
        } => {
            let repo = Repository::open(&root)?;
            if let Some(input) = input {
                enqueue(&repo, read_events(&input)?)?;
            }
            run_queue(&repo, config.as_deref(), max_work as usize)
        }
        Command::Status { root } => {
            let repo = Repository::open(&root)?;
            Ok(
                json!({"schema":"codeclew-docs-update-status/1.0","targets":state(&repo)?.targets,"policies":policies(&repo)?,"inputDigest":repo.input_digest()?}),
            )
        }
    }
}
