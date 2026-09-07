//! One CLI surface for engineers and external agents; no embedded model or API key.
use super::{
    analysis, check, invalid,
    model::*,
    store::{self, Repository},
};
use crate::error::ClewError;
use clap::{Args, Subcommand};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Initialize a separate, user-owned documentation repository.
    Init {
        #[arg(long)]
        root: PathBuf,
        #[arg(long, default_value = "Service documentation")]
        title: String,
    },
    /// Register or inspect independently versioned services.
    Service {
        #[command(subcommand)]
        command: ServiceCommand,
    },
    /// Bind an existing local checkout without changing the service source.
    Bind {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        service: String,
        #[arg(long)]
        repo: PathBuf,
    },
    /// Record and inspect declared HTTP relationships.
    Interaction {
        #[command(subcommand)]
        command: InteractionCommand,
    },
    /// Rebuild current source evidence and report affected document fragments.
    Check(ListArgs),
    /// Read bounded source-backed authoring input. Use --refresh to rebuild evidence.
    Context(ContextArgs),
    /// Render the overview, each service, and named interaction scenarios offline.
    Render {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        input: Vec<PathBuf>,
        #[arg(long)]
        require_complete: bool,
    },
}
#[derive(Debug, Args)]
pub struct RootArgs {
    #[arg(long)]
    pub root: PathBuf,
}
#[derive(Debug, Args)]
pub struct InputArgs {
    #[arg(long)]
    pub root: PathBuf,
    #[arg(long)]
    pub input: PathBuf,
    /// Copy inputDigest from the latest catalogue inspection to prevent lost updates.
    #[arg(long)]
    pub expected_input_digest: Option<String>,
}
#[derive(Debug, Args)]
pub struct ListArgs {
    #[arg(long)]
    pub root: PathBuf,
    #[arg(long)]
    pub cursor: Option<String>,
    #[arg(long,default_value_t=20,value_parser=clap::value_parser!(u32).range(1..=100))]
    pub limit: u32,
}
#[derive(Debug, Subcommand)]
pub enum ServiceCommand {
    Add(InputArgs),
    List(ListArgs),
    Show {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        id: String,
    },
}
#[derive(Debug, Subcommand)]
pub enum InteractionCommand {
    Put(InputArgs),
    List(ListArgs),
    Show {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        id: String,
    },
    Remove {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        id: String,
        #[arg(long)]
        expected_input_digest: String,
    },
    Candidates {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        input: PathBuf,
    },
}
#[derive(Debug, Args)]
pub struct ContextArgs {
    #[arg(long)]
    pub root: PathBuf,
    #[arg(
        long,
        required_unless_present = "scenario",
        conflicts_with = "scenario"
    )]
    pub service: Option<String>,
    #[arg(long)]
    pub scenario: Option<String>,
    #[arg(long, requires = "service")]
    pub entrypoint: Option<String>,
    #[arg(long)]
    pub refresh: bool,
    #[arg(long)]
    pub cursor: Option<String>,
    #[arg(long,default_value_t=20,value_parser=clap::value_parser!(u32).range(1..=100))]
    pub limit: u32,
}

fn inspect<T: serde::Serialize>(
    repo: &Repository,
    rows: BTreeMap<String, T>,
    args: &ListArgs,
) -> Result<Value, ClewError> {
    let digest = repo.input_digest()?;
    page(
        &super::digest(&(digest.clone(), std::any::type_name::<T>()))?,
        rows.into_iter()
            .map(|(id, record)| json!({"id":id,"record":record}))
            .collect(),
        args.cursor.as_deref(),
        args.limit as usize,
        json!({"inputDigest":digest}),
    )
}

pub fn run(command: Command) -> Result<Value, ClewError> {
    match command {
        Command::Init { root, title } => Repository::init(&root, &title),
        Command::Bind {
            root,
            service,
            repo,
        } => analysis::bind(&Repository::open(&root)?, &service, &repo),
        Command::Service { command } => match command {
            ServiceCommand::Add(args) => Repository::open(&args.root)?.service_add(
                store::read(&args.input, store::MAX_RECORD)?,
                args.expected_input_digest.as_deref(),
            ),
            ServiceCommand::List(args) => {
                let repo = Repository::open(&args.root)?;
                inspect(&repo, repo.services()?, &args)
            }
            ServiceCommand::Show { root, id } => {
                let repo = Repository::open(&root)?;
                let rows = repo.services()?;
                Ok(
                    json!({"schema":"codeclew-docs-record/1.0","inputDigest":repo.input_digest()?,"record":rows.get(&id).ok_or_else(||invalid("unknown service"))?}),
                )
            }
        },
        Command::Interaction { command } => match command {
            InteractionCommand::Put(args) => Repository::open(&args.root)?.interaction_put(
                store::read(&args.input, store::MAX_RECORD)?,
                args.expected_input_digest.as_deref(),
            ),
            InteractionCommand::List(args) => {
                let repo = Repository::open(&args.root)?;
                inspect(&repo, repo.interactions()?, &args)
            }
            InteractionCommand::Show { root, id } => {
                let repo = Repository::open(&root)?;
                let rows = repo.interactions()?;
                Ok(
                    json!({"schema":"codeclew-docs-record/1.0","inputDigest":repo.input_digest()?,"record":rows.get(&id).ok_or_else(||invalid("unknown interaction"))?}),
                )
            }
            InteractionCommand::Remove {
                root,
                id,
                expected_input_digest,
            } => Repository::open(&root)?.interaction_remove(&id, &expected_input_digest),
            InteractionCommand::Candidates { root, input } => {
                let repo = Repository::open(&root)?;
                let i: Interaction = store::read(&input, store::MAX_RECORD)?;
                store::endpoint(&i.from, &repo.services()?)?;
                store::endpoint(&i.to, &repo.services()?)?;
                let checked = check::run(&repo)?;
                checked.save(&repo)?;
                Ok(
                    json!({"schema":"codeclew-docs-candidates/1.0","inputDigest":checked.input_digest,"result":check::check_interaction(&i,&checked.services)?,"unresolved":checked.unresolved}),
                )
            }
        },
        Command::Check(args) => {
            let repo = Repository::open(&args.root)?;
            let checked = check::run(&repo)?;
            checked.save(&repo)?;
            let mut value = checked.summary();
            value["freshness"] = super::bindings::freshness(
                super::bindings::baseline(&repo)?.as_ref().map(|(_, b)| b),
                &checked,
            );
            if super::bytes(&value)?.len() <= 48 * 1024 && args.cursor.is_none() {
                return Ok(value);
            }
            let status = value["freshness"]["status"].clone();
            let mut rows = Vec::new();
            for (section, record) in value.as_object().unwrap() {
                match record {
                    Value::Object(entries) => {
                        for (id, entry) in entries {
                            if let Some(entries) = entry.as_array() {
                                for (index, item) in entries.iter().enumerate() {
                                    rows.push(json!({"section":section,"id":format!("{id}/{index}"),"record":item}));
                                }
                            } else {
                                rows.push(json!({"section":section,"id":id,"record":entry}));
                            }
                        }
                    }
                    _ => rows.push(json!({"section":section,"id":section,"record":record})),
                }
            }
            page(
                &checked.context_digest,
                rows,
                args.cursor.as_deref(),
                args.limit as usize,
                json!({"reportSchema":"codeclew-documentation-check/1.0","inputDigest":checked.input_digest,"contextDigest":checked.context_digest,"freshness":{"status":status}}),
            )
        }
        Command::Context(args) => context(args),
        Command::Render {
            root,
            input,
            require_complete,
        } => {
            let narratives = input
                .iter()
                .map(|p| store::read(p, store::MAX_RECORD))
                .collect::<Result<Vec<Narrative>, _>>()?;
            super::render::publish(&Repository::open(&root)?, narratives, require_complete)
        }
    }
}

pub fn page(
    binding: &str,
    items: Vec<Value>,
    cursor: Option<&str>,
    limit: usize,
    mut meta: Value,
) -> Result<Value, ClewError> {
    let prefix = binding.trim_start_matches("sha256:");
    let start = match cursor {
        None => 0,
        Some(value) => {
            let (digest, offset) = value
                .split_once(':')
                .ok_or_else(|| invalid("invalid documentation cursor"))?;
            if digest != prefix {
                return Err(invalid("documentation cursor belongs to changed input"));
            }
            offset
                .parse::<usize>()
                .map_err(|_| invalid("invalid documentation cursor offset"))?
        }
    };
    if start > items.len() {
        return Err(invalid("documentation cursor is out of range"));
    }
    let total = items.len();
    let mut out = Vec::new();
    let mut omitted = Vec::new();
    let mut consumed = start;
    let mut used = super::bytes(&meta)?.len();
    if used > 8 * 1024 {
        return Err(invalid(
            "documentation page metadata exceeds stdout budget; inspect unresolved service records separately",
        ));
    }
    for item in items.into_iter().skip(start).take(limit) {
        let size = super::bytes(&item)?.len();
        if size > 48 * 1024 {
            omitted.push(
                json!({"index":consumed,"reason":"ITEM_EXCEEDS_STDOUT_BUDGET","id":item.get("id")}),
            );
            consumed += 1;
            continue;
        }
        if used + size > 56 * 1024 {
            break;
        }
        used += size;
        out.push(item);
        consumed += 1;
    }
    meta["schema"] = json!("codeclew-docs-page/1.0");
    meta["items"] = json!(out);
    meta["omitted"] = json!(omitted);
    meta["total"] = json!(total);
    meta["nextCursor"] = if consumed < total {
        json!(format!("{prefix}:{consumed}"))
    } else {
        Value::Null
    };
    Ok(meta)
}

fn context(args: ContextArgs) -> Result<Value, ClewError> {
    let repo = Repository::open(&args.root)?;
    let path = repo.path(".codeclew/cache/latest-check.json")?;
    let (checked, authority) = if !args.refresh && path.exists() {
        let checked: check::Check = store::read(&path, 64 * 1024 * 1024)?;
        if checked.input_digest != repo.input_digest()? {
            return Err(invalid(
                "documentation declarations changed; rerun context --refresh",
            ));
        }
        (checked, "RETAINED_CHECK_NOT_REVERIFIED")
    } else {
        let checked = check::run(&repo)?;
        checked.save(&repo)?;
        (checked, "CURRENT_SOURCE_CHECK")
    };
    let subject = if let Some(id) = &args.service {
        format!("service:{id}")
    } else {
        format!("scenario:{}", args.scenario.as_deref().unwrap_or(""))
    };
    let mut selected = BTreeSet::new();
    let mut items = Vec::new();
    if let Some(id) = &args.service {
        let e = checked
            .services
            .get(id)
            .ok_or_else(|| invalid("service evidence is unavailable; inspect docs check"))?;
        let entries: Vec<_> = e
            .entrypoints
            .iter()
            .filter(|entry| args.entrypoint.as_ref().is_none_or(|id| &entry.id == id))
            .collect();
        if args.entrypoint.is_some() && entries.is_empty() {
            return Err(invalid("unknown entrypoint"));
        }
        for entry in &entries {
            items.push(json!({"kind":"ENTRYPOINT","id":entry.id,"record":entry}));
            if args.entrypoint.is_some() {
                selected.extend(entry.dependency_ids.iter().cloned());
            }
        }
        if args.entrypoint.is_some() {
            let symbols: BTreeSet<_> = entries.iter().map(|e| e.symbol.as_str()).collect();
            selected.extend(
                e.observations
                    .values()
                    .filter(|o| {
                        symbols.contains(o.symbol.as_str()) || o.kind.starts_with("CONTRACT")
                    })
                    .map(|o| o.id.clone()),
            );
            // Follow bounded local bodies so an agent can see the callee evidence too.
            for _ in 0..4 {
                let targets: BTreeSet<String> = selected
                    .iter()
                    .filter_map(|id| checked.dependencies.get(id))
                    .filter_map(|o| o.normalized["target"].as_str().map(str::to_owned))
                    .collect();
                selected.extend(
                    e.observations
                        .values()
                        .filter(|o| targets.contains(&o.symbol))
                        .map(|o| o.id.clone()),
                );
                if selected.len() > 4096 {
                    return Err(invalid(
                        "entrypoint context exceeds bounded dependency scope",
                    ));
                }
            }
        }
    } else if let Some(id) = &args.scenario {
        let s = checked
            .scenarios
            .get(id)
            .ok_or_else(|| invalid("unknown scenario"))?;
        selected.extend(s.dependency_ids.iter().cloned());
        for step in &s.steps {
            items.push(json!({"kind":"FLOW_STEP","id":step.id,"record":step}));
        }
        items.push(
            json!({"kind":"COVERAGE","id":id,"boundaries":s.boundaries,"truncated":s.truncated}),
        );
    }
    if let Some((_, baseline)) = super::bindings::baseline(&repo)?
        && let Some(narrative) = baseline.narratives.get(&subject)
    {
        for operation in &narrative.operations {
            if args
                .entrypoint
                .as_ref()
                .is_none_or(|id| id == &operation.id)
            {
                items.push(json!({"kind":"RETAINED_OPERATION","id":operation.id,"record":operation,"authority":"RETAINED_NARRATIVE_NOT_REVERIFIED"}));
            }
        }
        items.push(json!({"kind":"RETAINED_GAPS","id":subject,"record":narrative.gaps}));
    }
    let sources = checked.sources();
    let mut source_ids = BTreeSet::new();
    for id in selected {
        if let Some(d) = checked.dependencies.get(&id) {
            source_ids.extend(d.source_ids.iter().cloned());
            items.push(json!({"kind":"DEPENDENCY","id":id,"record":d}));
        }
    }
    for id in source_ids {
        if let Some(source) = sources.get(&id) {
            items.push(json!({"kind":"SOURCE","id":id,"record":source}));
        }
    }
    page(
        &super::digest(&(
            checked.context_digest.clone(),
            subject.clone(),
            args.entrypoint.clone(),
        ))?,
        items,
        args.cursor.as_deref(),
        args.limit as usize,
        json!({"subject":subject,"inputDigest":checked.input_digest,"contextDigest":checked.context_digest,"authority":authority,"narrativeAuthority":"AGENT_INFERRED","unresolved":checked.unresolved}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cursor_binds_input_and_does_not_drop_items_at_page_boundaries() {
        let items = (0..5).map(|id| json!({"id":id})).collect::<Vec<_>>();
        let first = page("sha256:bound", items.clone(), None, 2, json!({})).unwrap();
        assert_eq!(first["items"].as_array().unwrap().len(), 2);
        let second = page(
            "sha256:bound",
            items.clone(),
            first["nextCursor"].as_str(),
            2,
            json!({}),
        )
        .unwrap();
        assert_eq!(second["items"][0]["id"], 2);
        assert!(
            page(
                "sha256:changed",
                items,
                first["nextCursor"].as_str(),
                2,
                json!({})
            )
            .is_err()
        );
    }
    #[test]
    fn oversized_metadata_cannot_create_a_nonprogressing_cursor() {
        assert!(
            page(
                "sha256:bound",
                vec![json!({"id":0})],
                None,
                1,
                json!({"detail":"x".repeat(9*1024)})
            )
            .is_err()
        );
    }

    #[test]
    fn oversized_context_item_is_explicit_and_has_continuation() {
        let result = page(
            "sha256:bound",
            vec![
                json!({"id":"large","text":"x".repeat(64*1024)}),
                json!({"id":"next"}),
            ],
            None,
            1,
            json!({}),
        )
        .unwrap();
        assert_eq!(result["omitted"][0]["reason"], "ITEM_EXCEEDS_STDOUT_BUDGET");
        assert_eq!(result["nextCursor"], "bound:1");
    }
}
