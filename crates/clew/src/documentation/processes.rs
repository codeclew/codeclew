//! Explicit saved process definitions and bounded, reviewable child composition.
use super::{
    bindings,
    check::Check,
    cli::ListArgs,
    digest, invalid, io_error,
    model::*,
    store::{self, Repository},
    work,
};
use crate::error::ClewError;
use clap::Subcommand;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

pub const OVERVIEW: &str = "process-overview";
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Details {
    pub scope: String,
    pub participants: Vec<String>,
    #[serde(default)]
    pub objects: Vec<String>,
    pub trigger: String,
    pub outcomes: Vec<String>,
    #[serde(default)]
    pub linked_subviews: Vec<String>,
}
pub fn validate(s: &Scenario, services: &BTreeMap<String, Service>) -> Result<(), ClewError> {
    let Some(p) = &s.process else {
        return if s.schema == "codeclew-documentation-process/1.0" {
            Err(invalid("saved process requires explicit process metadata"))
        } else {
            Ok(())
        };
    };
    let bounded = |v: &str| !v.trim().is_empty() && v.len() <= 2048;
    if s.schema != "codeclew-documentation-process/1.0"
        || !bounded(&s.title)
        || !bounded(&s.summary)
        || !bounded(&p.scope)
        || !bounded(&p.trigger)
        || p.participants.is_empty()
        || p.participants.len() > 64
        || p.objects.len() > 64
        || p.outcomes.is_empty()
        || p.outcomes.len() > 32
        || !p.outcomes.iter().all(|s| bounded(s))
        || p.linked_subviews.len() > 32
        || !p.participants.contains(&s.root.service)
        || p.participants.iter().any(|id| !services.contains_key(id))
        || p.objects
            .iter()
            .any(|id| !id.strip_prefix("entity:").is_some_and(store::valid_id))
        || p.linked_subviews.iter().any(|id| !store::valid_id(id))
        || [&p.participants, &p.objects, &p.linked_subviews]
            .iter()
            .any(|v| v.iter().collect::<BTreeSet<_>>().len() != v.len())
    {
        return Err(invalid(
            "invalid saved process scope, participants, objects, outcomes or linked subview bounds",
        ));
    }
    Ok(())
}
pub fn expected(checked: &Check, id: &str) -> BTreeSet<String> {
    let mut ids = BTreeSet::from([id.into()]);
    if checked.dependencies.contains_key(&format!("process:{id}")) {
        ids.insert(OVERVIEW.into());
    }
    ids
}
pub fn overview(checked: &Check, subject: &str, root: &str) -> bool {
    root == OVERVIEW
        && subject
            .strip_prefix("scenario:")
            .is_some_and(|id| checked.dependencies.contains_key(&format!("process:{id}")))
}
#[derive(Debug, Subcommand)]
pub enum Command {
    List {
        #[command(flatten)]
        page: ListArgs,
    },
    Show {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        id: String,
    },
    Put {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        expected_input_digest: String,
    },
    /// Inspect a candidate definition without saving it or running agents.
    Inspect {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        input: PathBuf,
    },
    Prepare {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        id: String,
        #[arg(long)]
        overview: bool,
    },
}
pub fn run(command: Command) -> Result<Value, ClewError> {
    let root = match &command {
        Command::List { page } => &page.root,
        Command::Show { root, .. }
        | Command::Put { root, .. }
        | Command::Inspect { root, .. }
        | Command::Prepare { root, .. } => root,
    };
    let repo = Repository::open(root)?;
    match command {
        Command::List { page } => {
            let records=repo.scenarios()?.into_values().map(|s|json!({"id":s.id,"title":s.title,"kind":if s.process.is_some(){"SAVED_PROCESS"}else{"LEGACY_SCENARIO"},"subject":format!("scenario:{}",s.id)})).collect::<Vec<_>>();
            super::cli::page(
                &digest(&records)?,
                records,
                page.cursor.as_deref(),
                page.limit as usize,
                json!({"inputDigest":repo.input_digest()?}),
            )
        }
        Command::Show { id, .. } => Ok(
            json!({"definition":repo.scenarios()?.get(&id).ok_or_else(||invalid("unknown process"))?,"inputDigest":repo.input_digest()?}),
        ),
        Command::Put {
            input,
            expected_input_digest,
            ..
        } => {
            let definition = load_definition(&repo, &input)?;
            repo.put(
                &format!("scenarios/{}.yaml", definition.id),
                &definition,
                Some(&expected_input_digest),
            )
        }
        Command::Inspect { input, .. } => {
            let definition = load_definition(&repo, &input)?;
            let mut scenarios = repo.scenarios()?;
            scenarios.insert(definition.id.clone(), definition.clone());
            let captured = super::check::run(&repo)?;
            let checked = super::check::assemble(
                captured.input_digest,
                captured.services,
                captured.unresolved,
                &repo.interactions()?,
                &scenarios,
            )?;
            Ok(
                json!({"status":"TRANSIENT","saved":false,"definition":definition,"context":checked.scenarios[&definition.id],"linkedSubviews":definition.process.as_ref().map(|p|&p.linked_subviews),"limitation":"Linked child explanations are checked only for an explicitly saved definition."}),
            )
        }
        Command::Prepare { id, overview, .. } => {
            if !repo.scenarios()?.contains_key(&id) {
                return Err(invalid("unknown process"));
            }
            work::prepare(&repo,format!("scenario:{id}"),serde_json::from_value(json!({"schema":"codeclew-documentation-work-request/1.0","audience":"Process maintainers and architecture readers","entrypoint":overview.then_some(OVERVIEW),"maxItems":20,"maxBytes":40960})).map_err(io_error)?)
        }
    }
}
fn load_definition(repo: &Repository, path: &std::path::Path) -> Result<Scenario, ClewError> {
    let s: Scenario = store::read(path, 256 * 1024)?;
    if s.schema != "codeclew-documentation-process/1.0"
        || !store::valid_id(&s.id)
        || s.id == OVERVIEW
        || s.max_depth > 16
        || s.max_nodes == 0
        || s.max_nodes > 512
    {
        return Err(invalid("invalid process identity or traversal bounds"));
    }
    validate(&s, &repo.services()?)?;
    store::endpoint(&s.root, &repo.services()?)?;
    let interactions = repo.interactions()?;
    if s.interactions.iter().collect::<BTreeSet<_>>().len() != s.interactions.len()
        || s.interactions
            .iter()
            .any(|id| !interactions.contains_key(id))
    {
        return Err(invalid("process has a dangling or duplicate interaction"));
    }
    let entities = super::entities::records(repo)?;
    if s.process
        .as_ref()
        .unwrap()
        .objects
        .iter()
        .any(|id| !entities.contains_key(&id[7..]))
    {
        return Err(invalid(
            "process object must name an existing explicit entity identity",
        ));
    }
    Ok(s)
}
fn observation(
    checked: &mut Check,
    id: String,
    kind: &str,
    value: Value,
    sources: Vec<String>,
) -> Result<(), ClewError> {
    checked.dependencies.insert(
        id.clone(),
        Observation {
            id: id.clone(),
            kind: kind.into(),
            service: String::new(),
            symbol: id,
            digest: digest(&value)?,
            normalized: value,
            source_ids: sources,
        },
    );
    Ok(())
}
/// Components are evaluated in dependency order. Cyclic/unavailable children never supply prose.
pub fn attach(repo: &Repository, checked: &mut Check) -> Result<(), ClewError> {
    let baseline = bindings::baseline(repo)?.map(|(_, b)| b);
    attach_versions(repo, checked, baseline.as_ref())
}
pub(super) fn attach_versions(
    repo: &Repository,
    checked: &mut Check,
    baseline: Option<&bindings::Bindings>,
) -> Result<(), ClewError> {
    let definitions = repo.scenarios()?;
    checked.dependencies.retain(|_, d| {
        !matches!(
            d.kind.as_str(),
            "PROCESS_DEFINITION" | "PROCESS_COMPONENT" | "PROCESS_SCOPE"
        )
    });
    for context in checked.scenarios.values_mut() {
        context.dependency_ids.retain(|id| {
            !id.starts_with("process:")
                && !id.starts_with("process-component:")
                && !id.starts_with("process-scope:")
        });
        context.boundaries.retain(|b| {
            !b.starts_with("LINKED_PROCESS_") && !b.starts_with("PROCESS_PARTICIPANT_UNAVAILABLE:")
        });
    }
    let membership: Vec<_> = repo.interactions()?.into_keys().collect();
    for (id, s) in &definitions {
        let Some(p) = &s.process else {
            continue;
        };
        let mut deps = vec![format!("scenario:{id}")];
        deps.extend(p.objects.iter().cloned());
        let missing_objects: Vec<_> = p
            .objects
            .iter()
            .filter(|id| !checked.dependencies.contains_key(*id))
            .collect();
        observation(
            checked,
            format!("process:{id}"),
            "PROCESS_DEFINITION",
            json!({"definition":s,"dependencyIds":deps,"missingObjects":missing_objects,"authority":"EXPLICIT_SAVED_REQUEST_NOT_RUNTIME_PROOF"}),
            vec![],
        )?;
        let deps: Vec<_> = checked
            .dependencies
            .values()
            .filter(|d| {
                p.participants.contains(&d.service)
                    && matches!(
                        d.kind.as_str(),
                        "SOURCE_SCOPE"
                            | "MODULE_SCOPE"
                            | "CONTRACT_SCOPE"
                            | "ENTITY_SCOPE"
                            | "NOTE_SCOPE"
                    )
            })
            .map(|d| d.id.clone())
            .collect();
        let unavailable: Vec<_> = p
            .participants
            .iter()
            .filter(|id| !checked.services.contains_key(*id))
            .cloned()
            .collect();
        observation(
            checked,
            format!("process-scope:{id}"),
            "PROCESS_SCOPE",
            json!({"interactionMembership":membership,"participants":p.participants,"unavailableParticipants":unavailable,"dependencyIds":deps,"runtime":"UNKNOWN"}),
            vec![],
        )?;
        if let Some(context) = checked.scenarios.get_mut(id) {
            context
                .dependency_ids
                .extend([format!("process:{id}"), format!("process-scope:{id}")]);
            context.boundaries.extend(
                unavailable
                    .iter()
                    .map(|id| format!("PROCESS_PARTICIPANT_UNAVAILABLE:{id}")),
            );
        }
    }
    for id in definitions.keys() {
        let mut active = BTreeSet::new();
        let mut visited = BTreeSet::new();
        components(
            id,
            &definitions,
            baseline,
            checked,
            &mut active,
            &mut visited,
            0,
        )?;
    }
    // Complete links after traversal: a back edge must keep the dependencies
    // that were not yet available while its ancestor was active.
    for d in checked
        .dependencies
        .values_mut()
        .filter(|d| d.kind == "PROCESS_COMPONENT")
    {
        let child = d.normalized["child"].as_str().unwrap();
        let mut deps: BTreeSet<String> =
            serde_json::from_value(d.normalized["dependencyIds"].clone()).map_err(io_error)?;
        if let Some(p) = definitions.get(child).and_then(|s| s.process.as_ref()) {
            deps.extend(
                p.linked_subviews
                    .iter()
                    .map(|id| format!("process-component:{child}:{id}")),
            );
        }
        d.normalized["dependencyIds"] = json!(deps);
        d.digest = digest(&d.normalized)?;
    }
    checked.refresh_digest()
}
fn components(
    id: &str,
    defs: &BTreeMap<String, Scenario>,
    baseline: Option<&bindings::Bindings>,
    checked: &mut Check,
    active: &mut BTreeSet<String>,
    visited: &mut BTreeSet<String>,
    depth: usize,
) -> Result<(), ClewError> {
    let Some(p) = defs.get(id).and_then(|s| s.process.as_ref()) else {
        return Ok(());
    };
    if !active.insert(id.into()) {
        return Ok(());
    }
    visited.insert(id.into());
    for child in &p.linked_subviews {
        let key = format!("process-component:{id}:{child}");
        if checked.dependencies.contains_key(&key) {
            continue;
        }
        let mut gap = if active.contains(child) {
            Some("LINKED_PROCESS_CYCLE")
        } else if depth >= 16 || visited.len() >= 64 {
            Some("LINKED_PROCESS_BUDGET_EXHAUSTED")
        } else if !defs.contains_key(child) {
            Some("LINKED_PROCESS_MISSING")
        } else {
            None
        };
        if gap.is_none() {
            components(child, defs, baseline, checked, active, visited, depth + 1)?;
        }
        let child_subject = format!("scenario:{child}");
        let root = if defs.get(child).is_some_and(|s| s.process.is_some()) {
            OVERVIEW
        } else {
            child.as_str()
        };
        let operation = baseline
            .and_then(|b| b.narratives.get(&child_subject))
            .and_then(|n| n.operations.iter().find(|o| o.id == root));
        let version =
            baseline.and_then(|b| b.accepted_versions.get(&format!("{child_subject}/{root}")));
        let mut deps = BTreeSet::from([format!("scenario:{child}")]);
        if let Some(context) = checked.scenarios.get(child) {
            deps.extend(context.dependency_ids.iter().cloned());
        }
        let mut sources = vec![];
        let mut accepted = Value::Null;
        if gap.is_none() {
            match (operation, version) {
                (Some(o), Some(v))
                    if v.operation_digest == digest(o)?
                        && v.verification.starts_with("VERIFIED")
                        && v.source_revisions.iter().all(|(id, revision)| {
                            checked
                                .services
                                .get(id)
                                .is_some_and(|s| &s.revision == revision)
                        })
                        && v.influence.iter().all(|(id, d)| {
                            checked.dependencies.get(id).is_some_and(|o| &o.digest == d)
                        }) =>
                {
                    deps.extend(v.influence.keys().cloned());
                    sources = o.summary.source_ids.clone();
                    if sources.is_empty()
                        || sources.iter().any(|id| !checked.sources().contains_key(id))
                    {
                        gap = Some("LINKED_PROCESS_SOURCES_UNAVAILABLE");
                        sources.clear();
                    } else {
                        accepted = json!({"summary":o.summary,"claimVersion":digest(o)?,"acceptedVersion":digest(v)?,"definitionVersion":checked.dependencies.get(&format!("scenario:{child}")).map(|d|&d.digest),"sourceInfluence":v.influence,"sourceRevisions":v.source_revisions,"verification":v.verification,"limitations":v.limitations,"boundaries":o.boundaries});
                    }
                }
                (Some(_), Some(_)) => gap = Some("LINKED_PROCESS_STALE"),
                _ => gap = Some("LINKED_PROCESS_UNASSESSED"),
            }
        }
        observation(
            checked,
            key.clone(),
            "PROCESS_COMPONENT",
            json!({"parent":id,"child":child,"href":format!("{child}.html"),"status":if gap.is_some(){"GAP"}else{"ACCEPTED_CHILD"},"gap":gap,"accepted":accepted,"dependencyIds":deps,"authority":"REVIEWED_CHILD_INTERPRETATION_NOT_RUNTIME_PROOF"}),
            sources,
        )?;
        if let Some(context) = checked.scenarios.get_mut(id) {
            context.dependency_ids.push(key);
            if let Some(gap) = gap {
                context.boundaries.push(format!("{gap}:{child}"));
            }
        }
    }
    active.remove(id);
    Ok(())
}
pub fn page(checked: &Check, subject: &str) -> Value {
    let Some(id) = subject.strip_prefix("scenario:") else {
        return Value::Null;
    };
    let Some(def) = checked.dependencies.get(&format!("process:{id}")) else {
        return Value::Null;
    };
    json!({"definition":def.normalized["definition"],"definitionVersion":def.digest,"scope":checked.dependencies.get(&format!("process-scope:{id}")),"linkedSubviews":checked.dependencies.values().filter(|d|d.kind=="PROCESS_COMPONENT"&&d.normalized["parent"]==id).map(|d|&d.normalized).collect::<Vec<_>>(),"authority":"Explicit saved request; generated interpretation is reviewed separately."})
}
pub fn mark_targets(data: &mut Value, checked: &Check, subject: &str) {
    if data["process"].is_null() {
        return;
    }
    let current = page(checked, subject);
    let mut retained = data["process"].clone();
    if let Some(object) = retained.as_object_mut() {
        object.remove("targetChanged");
        object.remove("targetGaps");
    }
    data["process"]["targetChanged"] = json!(retained != current);
    data["process"]["targetGaps"] = json!(
        checked
            .scenarios
            .get(subject.strip_prefix("scenario:").unwrap_or(""))
            .map(|s| &s.boundaries)
    );
}
pub fn markdown(process: &Value) -> String {
    if process.is_null() {
        return String::new();
    }
    let esc = |v: &Value| super::render::escape(v.as_str().unwrap_or(""));
    let d = &process["definition"]["process"];
    let mut out = format!(
        "\n## Saved process definition\n\nExplicit requested scope and outcomes; source support is assessed separately.\n\nScope: {}\n\nTrigger: {}\n\n",
        esc(&d["scope"]),
        esc(&d["trigger"])
    );
    if process["targetChanged"] == true {
        out.push_str("The captured definition or linked children have changed; retained explanations need review.\n\n");
    }
    for key in ["participants", "objects", "outcomes"] {
        out.push_str(&format!("{key}:\n\n"));
        for v in d[key].as_array().into_iter().flatten() {
            out.push_str(&format!("- {}\n", esc(v)));
        }
        out.push('\n');
    }
    out.push_str("### Linked child views: captured versions\n\n");
    for c in process["linkedSubviews"].as_array().into_iter().flatten() {
        out.push_str(&format!(
            "- [{}]({}.md): {}\n",
            esc(&c["child"]),
            esc(&c["child"]),
            esc(if c["gap"].is_null() {
                &c["accepted"]["verification"]
            } else {
                &c["gap"]
            })
        ));
        if let Some(summary) = c["accepted"]["summary"]["text"].as_str() {
            out.push_str(&format!("\n  {}\n", super::render::escape(summary)));
        }
    }
    out
}
