//! One CLI surface for engineers and external agents; no embedded model or API key.
use super::{
    analysis, check, invalid,
    model::*,
    store::{self, Repository},
};
use crate::error::ClewError;
use clap::{Args, Subcommand};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

const CHECK_REPORT_SCHEMA: &str = "codeclew-documentation-check-report/1.0";

/// Small mutable-independent binding for a paginated check report. The heavy
/// check is kept in Check's immutable snapshot; this file binds its identity,
/// selection and rendered freshness output without copying report rows.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CheckReportBinding {
    schema: String,
    report_id: String,
    snapshot: String,
    input_digest: String,
    context_digest: String,
    service_selection: Vec<String>,
    freshness: Value,
    output_binding: String,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Name and retain saved documentation evidence without copying or recapturing it.
    Snapshot {
        #[command(subcommand)]
        command: super::snapshot_pins::Command,
    },
    /// Rebuild declarations over an original saved capture without running analyzers or changing latest.
    Recompose {
        #[arg(long)]
        root: PathBuf,
        /// Immutable handle returned by an original docs check.
        #[arg(long)]
        snapshot: String,
    },
    /// Inspect immutable documentation snapshots without rerunning producers or agents.
    History {
        #[command(subcommand)]
        command: super::history::Command,
    },
    /// Reconcile exact target revisions and process bounded documentation updates.
    Update {
        #[command(subcommand)]
        command: super::updates::Command,
    },
    /// Capture, inspect and admit portable per-service evidence.
    Evidence {
        #[command(subcommand)]
        command: super::evidence_package::Command,
    },
    /// Compact validated legacy advisory cache metadata.
    Cache {
        #[command(subcommand)]
        command: super::sidecar_compaction::Command,
    },
    /// Manage explicitly saved evidence-bound entity views.
    View {
        #[command(subcommand)]
        command: super::dataflow::Command,
    },
    /// Save explicit process definitions and prepare maintained views.
    Process {
        #[command(subcommand)]
        command: super::processes::Command,
    },
    /// Inspect and author required service sections.
    Note {
        #[command(subcommand)]
        command: super::notes::Command,
    },
    Section {
        #[command(subcommand)]
        command: super::sections::Command,
    },
    /// Manage declared domain identities and proposed relationships.
    Entity {
        #[command(subcommand)]
        command: super::entities::Command,
    },
    /// Inspect built-in evidence capabilities and explicit service selection.
    Modules {
        #[command(subcommand)]
        command: super::modules::Command,
    },
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
    Check(CheckArgs),
    /// Prepare and read bounded immutable authoring work.
    Work {
        #[command(subcommand)]
        command: super::work::Command,
    },
    /// Submit constrained content for deterministic checks and separate review.
    Proposal {
        #[command(subcommand)]
        command: super::proposals::Command,
    },
    /// Publish current freshness while retaining previously accepted explanations.
    Refresh {
        #[arg(long)]
        root: PathBuf,
        #[arg(long, required = true)]
        status_only: bool,
    },
    /// Read saved source-backed authoring input. Explicit --refresh rebuilds evidence.
    Context(ContextArgs),
    /// Review affected claims with bounded before/after evidence.
    Changes(ChangeArgs),
    /// Render saved evidence offline; use --refresh to acquire current source evidence.
    Render {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        input: Vec<PathBuf>,
        #[arg(long)]
        require_complete: bool,
        /// Acquire current evidence with docs check before publishing; may run analyzers.
        #[arg(long, conflicts_with = "snapshot")]
        refresh: bool,
        /// Select saved evidence; defaults to latest saved check, never runs analyzers.
        #[arg(long)]
        snapshot: Option<String>,
    },
}
#[derive(Debug, Args)]
pub struct CheckArgs {
    #[command(flatten)]
    pub page: ListArgs,
    /// Check selected services only; all others remain explicitly unverified.
    #[arg(long = "service")]
    pub services: Vec<String>,
    /// Existing caller-owned 0700 directory for private Maven failure output.
    #[arg(long)]
    pub debug_output: Option<PathBuf>,
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
        /// Select saved evidence; defaults to the latest saved check.
        #[arg(long)]
        snapshot: Option<String>,
    },
}
#[derive(Debug, Args)]
pub struct ChangeArgs {
    #[arg(long)]
    pub root: PathBuf,
    #[arg(long)]
    pub fragment: Option<String>,
    /// Compare this immutable snapshot; defaults to latest saved check.
    #[arg(long)]
    pub snapshot: Option<String>,
    #[arg(long)]
    pub cursor: Option<String>,
    #[arg(long, default_value_t=20, value_parser=clap::value_parser!(u32).range(1..=100))]
    pub limit: u32,
}
#[derive(Debug, Clone, Copy, clap::ValueEnum, PartialEq, Eq, serde::Serialize)]
pub enum ContextFormat {
    Raw,
    Compact,
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
    /// Exact compiler identity or qualified declaration name for an interface DTO.
    #[arg(long = "symbol", requires = "service", conflicts_with = "entrypoint")]
    pub symbols: Vec<String>,
    /// Drill into exact source IDs listed by a context or change dossier.
    #[arg(long = "source", requires = "service", conflicts_with_all = ["entrypoint", "symbols"])]
    pub source_ids: Vec<String>,
    #[arg(long = "dependency", requires = "service", conflicts_with_all = ["entrypoint", "symbols", "source_ids"])]
    pub dependency_ids: Vec<String>,
    #[arg(long, value_enum, default_value = "raw")]
    pub format: ContextFormat,
    #[arg(long, conflicts_with = "cursor")]
    pub refresh: bool,
    /// Read this immutable check snapshot instead of the latest check.
    #[arg(long, conflicts_with = "refresh")]
    pub snapshot: Option<String>,
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
        Command::Snapshot { command } => super::snapshot_pins::run(command),
        Command::View { command } => super::dataflow::run(command),
        Command::Evidence { command } => super::evidence_package::run(command),
        Command::Cache { command } => super::sidecar_compaction::run(command),
        Command::Update { command } => super::updates::run(command),
        Command::History { command } => super::history::run(command),
        Command::Process { command } => super::processes::run(command),
        Command::Note { command } => super::notes::run(command),
        Command::Section { command } => super::sections::run(command),
        Command::Entity { command } => super::entities::run(command),
        Command::Modules { command } => super::modules::run(command),
        Command::Work { command } => super::work::run(command),
        Command::Proposal { command } => super::proposals::run(command),
        Command::Refresh {
            root,
            status_only: _,
        } => super::status::refresh(&Repository::open(&root)?),
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
            InteractionCommand::Candidates {
                root,
                input,
                snapshot,
            } => {
                let repo = Repository::open(&root)?;
                let i: Interaction = store::read(&input, store::MAX_RECORD)?;
                store::endpoint(&i.from, &repo.services()?)?;
                store::endpoint(&i.to, &repo.services()?)?;
                let selected = BTreeSet::from([i.from.service.clone(), i.to.service.clone()]);
                let (checked, snapshot) =
                    check::Check::retained(&repo, snapshot.as_deref(), &selected)?;
                Ok(
                    json!({"schema":"codeclew-docs-candidates/1.0","snapshot":snapshot,"authority":"PINNED_SNAPSHOT_NOT_REVERIFIED","inputDigest":checked.input_digest,"result":check::check_interaction(&i,&checked.services)?,"unresolved":checked.unresolved}),
                )
            }
        },
        Command::Recompose { root, snapshot } => {
            let repo = Repository::open(&root)?;
            let (checked, derived) = super::composition::recompose(&repo, &snapshot)?;
            let mut value = checked.summary();
            value["snapshot"] = json!(derived);
            value["parentSnapshot"] = json!(snapshot);
            value["authority"] = json!("RECOMPOSED_DECLARATIONS_SOURCE_NOT_REVERIFIED");
            Ok(value)
        }
        Command::Check(request) => {
            let args = request.page;
            let requested_services: BTreeSet<String> = request.services.into_iter().collect();
            let repo = Repository::open(&args.root)?;
            if let Some(cursor) = args.cursor.as_deref() {
                return check_followup(&repo, cursor, args.limit as usize, &requested_services);
            }
            let debug_output = request
                .debug_output
                .as_deref()
                .map(crate::maven_diagnostics::DebugOutput::open)
                .transpose()?;
            let checked = check::run_selected_with_diagnostics(
                &repo,
                &requested_services,
                debug_output.as_ref(),
            )?;
            let snapshot = checked.save_snapshot(&repo)?;
            let mut value = checked.summary();
            value["freshness"] = super::bindings::freshness(
                super::bindings::baseline(&repo)?.as_ref().map(|(_, b)| b),
                &checked,
            );
            value["snapshot"] = json!(snapshot.clone());
            if super::bytes(&value)?.len() <= 48 * 1024 {
                return Ok(value);
            }
            let freshness = value["freshness"].clone();
            let output_binding =
                check_output_binding(&checked, &requested_services, &snapshot, &value)?;
            let service_selection = requested_services.iter().cloned().collect::<Vec<_>>();
            let report_id = super::digest(&(
                snapshot.clone(),
                service_selection.clone(),
                output_binding.clone(),
            ))?;
            let binding = CheckReportBinding {
                schema: CHECK_REPORT_SCHEMA.into(),
                report_id,
                snapshot,
                input_digest: checked.input_digest.clone(),
                context_digest: checked.context_digest.clone(),
                service_selection,
                freshness,
                output_binding,
            };
            save_check_report(&repo, &binding)?;
            check_page(&binding, check_rows(&value), None, args.limit as usize)
        }
        Command::Context(args) => context(args),
        Command::Changes(args) => changes(args),
        Command::Render {
            root,
            input,
            require_complete,
            refresh,
            snapshot,
        } => {
            let mut narratives = Vec::new();
            let mut failures = BTreeMap::new();
            for (index, path) in input.iter().enumerate() {
                match store::read::<Narrative>(path, store::MAX_RECORD) {
                    Ok(narrative) => narratives.push(narrative),
                    Err(error) => {
                        failures.insert(
                            format!("input-{index}"),
                            json!({"reason":error.code,"nextAction":error.message}),
                        );
                    }
                }
            }
            let repo = Repository::open(&root)?;
            if refresh {
                super::render::publish_from_current_source(
                    &repo,
                    narratives,
                    require_complete,
                    failures,
                )
            } else if let Some(snapshot) = snapshot {
                super::render::publish_from_snapshot(
                    &repo,
                    narratives,
                    require_complete,
                    failures,
                    &snapshot,
                )
            } else {
                super::render::publish_with_failures(&repo, narratives, require_complete, failures)
            }
        }
    }
}

fn check_rows(value: &Value) -> Vec<Value> {
    let mut rows = Vec::new();
    for (section, record) in value
        .as_object()
        .into_iter()
        .flat_map(|object| object.iter())
    {
        match record {
            Value::Object(entries) => {
                for (id, entry) in entries {
                    if let Some(entries) = entry.as_array() {
                        for (index, item) in entries.iter().enumerate() {
                            rows.push(json!({
                                "section": section,
                                "id": format!("{id}/{index}"),
                                "record": item
                            }));
                        }
                    } else {
                        rows.push(json!({"section":section,"id":id,"record":entry}));
                    }
                }
            }
            _ => rows.push(json!({"section":section,"id":section,"record":record})),
        }
    }
    rows
}

fn check_output_binding(
    checked: &check::Check,
    services: &BTreeSet<String>,
    snapshot: &str,
    value: &Value,
) -> Result<String, ClewError> {
    super::digest(&(
        checked.input_digest.clone(),
        checked.context_digest.clone(),
        services,
        snapshot,
        value,
    ))
}

fn check_report_path(repo: &Repository, report_id: &str) -> Result<PathBuf, ClewError> {
    let key = check_report_key(report_id)?;
    repo.path(&format!(".codeclew/cache/check-reports/{key}.json"))
}

fn check_report_key(report_id: &str) -> Result<&str, ClewError> {
    let key = report_id.strip_prefix("sha256:").unwrap_or(report_id);
    if key.len() != 64
        || !key
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(invalid("invalid documentation report identity"));
    }
    Ok(key)
}

fn save_check_report(repo: &Repository, binding: &CheckReportBinding) -> Result<(), ClewError> {
    let encoded = super::bytes(binding)?;
    if encoded.len() as u64 > store::MAX_RECORD {
        return Err(invalid(
            "documentation report binding exceeds its portable record budget",
        ));
    }
    repo.atomic(
        &format!(
            ".codeclew/cache/check-reports/{}.json",
            binding.report_id.trim_start_matches("sha256:")
        ),
        &encoded,
    )
}

fn load_check_report(repo: &Repository, report_id: &str) -> Result<CheckReportBinding, ClewError> {
    let path = check_report_path(repo, report_id)?;
    let binding: CheckReportBinding = store::read(&path, store::MAX_RECORD)?;
    if binding.schema != CHECK_REPORT_SCHEMA || binding.report_id != report_id {
        return Err(invalid(
            "documentation report identity does not match cursor",
        ));
    }
    let recomputed = super::digest(&(
        binding.snapshot.clone(),
        binding.service_selection.clone(),
        binding.output_binding.clone(),
    ))?;
    if recomputed != binding.report_id {
        return Err(invalid("documentation report binding is corrupt"));
    }
    Ok(binding)
}

fn check_cursor(report_id: &str, offset: usize) -> String {
    format!("check-v1|{report_id}|{offset}")
}

fn parse_check_cursor(cursor: &str) -> Result<(&str, usize), ClewError> {
    let mut parts = cursor.split('|');
    if parts.next() != Some("check-v1") {
        return Err(invalid("invalid documentation check cursor"));
    }
    let report_id = parts
        .next()
        .ok_or_else(|| invalid("invalid documentation check cursor identity"))?;
    let offset = parts
        .next()
        .ok_or_else(|| invalid("invalid documentation check cursor offset"))?
        .parse::<usize>()
        .map_err(|_| invalid("invalid documentation check cursor offset"))?;
    if parts.next().is_some() {
        return Err(invalid("invalid documentation check cursor"));
    }
    check_report_key(report_id)?;
    Ok((report_id, offset))
}

fn check_page(
    binding: &CheckReportBinding,
    rows: Vec<Value>,
    cursor: Option<&str>,
    limit: usize,
) -> Result<Value, ClewError> {
    let start = cursor
        .map(|value| parse_check_cursor(value).map(|(_, offset)| offset))
        .transpose()?
        .unwrap_or(0);
    let internal = format!(
        "{}:{start}",
        binding.report_id.trim_start_matches("sha256:")
    );
    let mut result = page(
        &binding.report_id,
        rows,
        Some(&internal),
        limit,
        json!({
            // The public report retains Check's schema and exit-status
            // contract; CHECK_REPORT_SCHEMA belongs to the saved cursor binding.
            "reportSchema": "codeclew-documentation-check/1.0",
            "reportId": binding.report_id,
            "snapshot": binding.snapshot,
            "inputDigest": binding.input_digest,
            "contextDigest": binding.context_digest,
            "serviceSelection": binding.service_selection,
            // Detailed freshness records are already paginated as rows.
            "freshness": {"status": binding.freshness["status"]},
            "outputBinding": binding.output_binding,
        }),
    )?;
    if let Some(next) = result["nextCursor"].as_str()
        && let Some((_, offset)) = next.split_once(':')
    {
        result["nextCursor"] = json!(check_cursor(
            &binding.report_id,
            offset
                .parse()
                .map_err(|_| invalid("invalid generated documentation cursor"))?
        ));
    }
    Ok(result)
}

fn check_followup(
    repo: &Repository,
    cursor: &str,
    limit: usize,
    requested_services: &BTreeSet<String>,
) -> Result<Value, ClewError> {
    let (report_id, _) = parse_check_cursor(cursor)?;
    let binding = load_check_report(repo, report_id)?;
    let bound_services: BTreeSet<_> = binding.service_selection.iter().cloned().collect();
    if !requested_services.is_empty() && requested_services != &bound_services {
        return Err(invalid(
            "documentation cursor belongs to a different service selection",
        ));
    }
    let checked = check::Check::load_snapshot(repo, &binding.snapshot)?;
    if checked.input_digest != binding.input_digest
        || checked.context_digest != binding.context_digest
    {
        return Err(invalid(
            "documentation snapshot identity does not match report binding",
        ));
    }
    let mut value = checked.summary();
    value["freshness"] = binding.freshness.clone();
    value["snapshot"] = json!(binding.snapshot);
    let output_binding =
        check_output_binding(&checked, &bound_services, &binding.snapshot, &value)?;
    if output_binding != binding.output_binding {
        return Err(invalid("documentation snapshot output binding changed"));
    }
    check_page(&binding, check_rows(&value), Some(cursor), limit)
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

fn context(mut args: ContextArgs) -> Result<Value, ClewError> {
    let repo = Repository::open(&args.root)?;
    let selected = args.service.iter().cloned().collect::<BTreeSet<_>>();
    let (checked, snapshot, authority) = if args.refresh {
        let checked = check::run_selected(&repo, &selected)?;
        let snapshot = checked.save_snapshot(&repo)?;
        (checked, snapshot, "CURRENT_SOURCE_CHECK")
    } else {
        let (checked, snapshot) =
            check::Check::retained(&repo, args.snapshot.as_deref(), &selected)?;
        (checked, snapshot, "PINNED_SNAPSHOT_NOT_REVERIFIED")
    };
    let subject = if let Some(id) = &args.service {
        format!("service:{id}")
    } else {
        format!("scenario:{}", args.scenario.as_deref().unwrap_or(""))
    };
    let baseline = super::bindings::baseline(&repo)?;
    let retained = baseline
        .as_ref()
        .and_then(|(_, b)| b.narratives.get(&subject));
    args.snapshot = Some(snapshot.clone());
    context_from(&checked, &args, retained, authority)
}

pub(super) fn context_from(
    checked: &check::Check,
    args: &ContextArgs,
    retained: Option<&Narrative>,
    authority: &str,
) -> Result<Value, ClewError> {
    let subject = args
        .service
        .as_ref()
        .map(|id| format!("service:{id}"))
        .unwrap_or_else(|| format!("scenario:{}", args.scenario.as_deref().unwrap_or("")));
    let items = context_items(checked, args, retained)?;

    page(
        &super::digest(&(
            checked.context_digest.clone(),
            args.snapshot.clone(),
            subject.clone(),
            args.entrypoint.clone(),
            args.symbols.clone(),
            args.source_ids.clone(),
            args.dependency_ids.clone(),
            args.format,
        ))?,
        items,
        args.cursor.as_deref(),
        args.limit as usize,
        json!({"subject":subject,"snapshot":args.snapshot,"inputDigest":checked.input_digest,"contextDigest":checked.context_digest,"authority":authority,"narrativeAuthority":"AGENT_INFERRED","unresolved":checked.unresolved}),
    )
}

pub(super) fn context_items(
    checked: &check::Check,
    args: &ContextArgs,
    retained: Option<&Narrative>,
) -> Result<Vec<Value>, ClewError> {
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
            .filter(|_| {
                args.symbols.is_empty()
                    && args.source_ids.is_empty()
                    && args.dependency_ids.is_empty()
            })
            .filter(|entry| args.entrypoint.as_ref().is_none_or(|id| &entry.id == id))
            .collect();
        if args.entrypoint.is_some() && entries.is_empty() {
            return Err(invalid("unknown entrypoint"));
        }
        if args.dependency_ids.len() > 8 {
            return Err(invalid("select at most eight dependencies"));
        }
        for dependency in &args.dependency_ids {
            if checked.dependencies.get(dependency).is_none_or(|d| {
                d.service != *id && !matches!(d.kind.as_str(), "DOMAIN_ENTITY" | "ENTITY_SCOPE")
            }) {
                return Err(invalid("unknown service dependency"));
            }
            selected.insert(dependency.clone());
        }
        for symbol in &args.symbols {
            let matches: Vec<_> = e
                .observations
                .values()
                .filter(|o| {
                    o.kind == "SYMBOL"
                        && (o.symbol == *symbol
                            || o.normalized["ownerIdentity"].as_str().is_some_and(|owner| {
                                let owner = owner
                                    .strip_prefix("class:")
                                    .or_else(|| owner.strip_prefix("package:"))
                                    .unwrap_or(owner)
                                    .replace('/', ".");
                                o.normalized["name"]
                                    .as_str()
                                    .is_some_and(|name| format!("{owner}.{name}") == *symbol)
                            })
                            || o.symbol
                                .split_once(':')
                                .map(|(_, name)| {
                                    name.split('#').next().unwrap_or(name).replace('/', ".")
                                })
                                .is_some_and(|name| name == *symbol))
                })
                .collect();
            if matches.len() != 1 {
                return Err(invalid(
                    "symbol must select one exact compiler identity or qualified declaration name",
                ));
            }
            selected.insert(matches[0].id.clone());
            selected.extend(
                e.observations
                    .values()
                    .filter(|o| {
                        o.kind == "SYMBOL" && o.normalized["ownerIdentity"] == matches[0].symbol
                    })
                    .map(|o| o.id.clone()),
            );
        }
        if args.symbols.len() > 8 || selected.len() > 4096 {
            return Err(invalid(
                "select at most eight interface declarations per context request",
            ));
        }
        items.push(json!({"kind":"COVERAGE","id":id,"revision":e.revision,"extractor":e.extractor,"runtimeMode":e.runtime_mode,"coverage":e.coverage,"boundaries":e.boundaries,"callAuthority":if e.extractor==SOURCE_EXTRACTOR{"SYNTAX_UNRESOLVED"}else{"PROVIDER_EVIDENCE"}}));
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
    if let Some(narrative) = retained {
        for operation in &narrative.operations {
            if !args.symbols.is_empty() {
                continue;
            }
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
    // Compact declarations may omit nested event copies only after the same
    // selected callable's flow records have been included independently.
    let selected_symbols: BTreeSet<_> = selected
        .iter()
        .filter_map(|id| checked.dependencies.get(id))
        .filter(|o| o.kind == "SYMBOL")
        .map(|o| o.symbol.as_str())
        .collect();
    let flow_ids: Vec<_> = checked
        .dependencies
        .values()
        .filter(|o| {
            matches!(o.kind.as_str(), "FLOW" | "SEMANTIC_SYMBOL")
                && selected_symbols.contains(o.symbol.as_str())
        })
        .map(|o| o.id.clone())
        .collect();
    selected.extend(flow_ids);
    let sources = checked.sources();
    if args.source_ids.len() > 8 {
        return Err(invalid("select at most eight source fragments"));
    }
    let mut source_ids = BTreeSet::new();
    for id in &args.source_ids {
        let source = sources
            .get(id)
            .ok_or_else(|| invalid("unknown source ID"))?;
        if args.service.as_ref() != Some(&source.service) {
            return Err(invalid("source belongs to a different service"));
        }
        source_ids.insert(id.clone());
    }
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
    if args.format == ContextFormat::Compact {
        items = compact(items);
    }
    Ok(items)
}

/// A deterministic projection of already selected evidence, never a new resolver.
fn compact(items: Vec<Value>) -> Vec<Value> {
    let sources: Vec<_> = items
        .iter()
        .filter(|i| i["kind"] == "SOURCE")
        .map(|i| i["record"].clone())
        .collect();
    items.into_iter().map(|mut item| {
        match item["kind"].as_str().unwrap_or("") {
            "DEPENDENCY" => {
                let record=&mut item["record"];
                if let Some(map)=record.as_object_mut() && let Some(digest)=map.remove("digest") { map.insert("fullRecordDigest".into(),digest); }
                let normalized=&mut record["normalized"];
                if let Some(map)=normalized.as_object_mut() {
                    map.remove("sourceTokens");
                    if let Some(documentation)=map.get_mut("documentation").and_then(Value::as_object_mut)
                        && let Some(events)=documentation.remove("events") { documentation.insert("eventCount".into(),json!(events.as_array().map_or(0,Vec::len))); }
                }
                if record["kind"] == "FLOW" && record["sourceIds"].as_array().is_some_and(|s|!s.is_empty()) {
                    record["normalized"].as_object_mut().map(|m|m.remove("text"));
                }
                item["projection"]=json!("COMPACT_EVIDENCE_1");
                item["fullRecord"]=json!({"command":"docs context","format":"raw","service":item["record"]["service"],"dependency":item["id"]});
            },
            "SOURCE" => {
                let source=&item["record"];
                let covering=sources.iter().filter(|other| other["id"]!=source["id"] && other["service"]==source["service"] && other["revision"]==source["revision"] && other["file"]==source["file"]
                    && other["startLine"].as_u64() <= source["startLine"].as_u64()
                    && other["endLine"].as_u64() >= source["endLine"].as_u64()
                    && other["text"].as_str().is_some_and(|t|source["text"].as_str().is_some_and(|s|t.contains(s)))
                    && (other["text"].as_str().map(str::len)>source["text"].as_str().map(str::len) || other["id"].as_str()<source["id"].as_str()))
                    .max_by_key(|other|other["text"].as_str().map(str::len));
                if let Some(covering)=covering {
                    item=json!({"kind":"SOURCE_ALIAS","id":source["id"],"coveredBy":covering["id"],"file":source["file"],"startLine":source["startLine"],"endLine":source["endLine"],"authority":source["authority"],"textDigest":source["textDigest"],"drill":{"service":source["service"],"source":source["id"],"format":"raw"}});
                }
            },
            _ => {},
        }
        item
    }).collect()
}

fn changes(args: ChangeArgs) -> Result<Value, ClewError> {
    let repo = Repository::open(&args.root)?;
    let (bundle, old) = super::bindings::baseline(&repo)?
        .ok_or_else(|| invalid("change dossier requires a published documentation baseline"))?;
    super::bindings::verify_outputs(&repo, &bundle, &old)?;
    let (checked, snapshot) =
        check::Check::retained(&repo, args.snapshot.as_deref(), &BTreeSet::new())?;
    let report = super::bindings::freshness(Some(&old), &checked);
    let mut items = Vec::new();
    let mut dependencies = BTreeSet::new();
    let mut source_ids = BTreeSet::new();
    let mut changed_files = BTreeSet::new();
    let selected_fragment = args.fragment.as_ref().and_then(|id| old.fragments.get(id));
    let before_observations = selected_fragment
        .and_then(|f| f.evidence.as_ref())
        .map(|e| &e.observations)
        .unwrap_or(&old.observations);
    let before_sources = selected_fragment
        .and_then(|f| f.evidence.as_ref())
        .map(|e| &e.sources)
        .unwrap_or(&old.retained_sources);
    let affected = report["affected"].as_array().cloned().unwrap_or_default();
    if args
        .fragment
        .as_ref()
        .is_some_and(|id| !old.fragments.contains_key(id))
    {
        return Err(invalid("unknown baseline fragment"));
    }
    for change in affected
        .iter()
        .filter(|c| args.fragment.as_ref().is_none_or(|id| c["fragment"] == *id))
    {
        let id = change["fragment"].as_str().unwrap();
        let fragment = &old.fragments[id];
        dependencies.extend(fragment.dependencies.keys().cloned());
        source_ids.extend(fragment.sources.keys().cloned());
        items.push(json!({"kind":"AFFECTED_CLAIM","id":id,"subject":fragment.subject,"oldClaim":fragment.content,"oldClaimDigest":fragment.content_digest,"reasons":change["reasons"],"affectedViewId":id,"authority":"RETAINED_CLAIM_REQUIRES_REVIEW","oldClaimAvailable":!fragment.content.is_null()}));
    }
    for id in dependencies {
        let before = before_observations.get(&id);
        let after = checked.dependencies.get(&id);
        if before.map(|o| &o.digest) == after.map(|o| &o.digest) {
            continue;
        }
        for fact in [before, after].into_iter().flatten() {
            source_ids.extend(fact.source_ids.iter().cloned());
        }
        if before.is_some_and(|o| o.kind == "SOURCE_SCOPE")
            || after.is_some_and(|o| o.kind == "SOURCE_SCOPE")
        {
            let a = before.and_then(|o| o.normalized["inventory"].as_object());
            let b = after.and_then(|o| o.normalized["inventory"].as_object());
            let paths: BTreeSet<_> = a
                .into_iter()
                .flat_map(|m| m.keys())
                .chain(b.into_iter().flat_map(|m| m.keys()))
                .collect();
            for path in paths {
                if a.and_then(|m| m.get(path)) != b.and_then(|m| m.get(path)) {
                    changed_files.insert((before.or(after).unwrap().service.clone(), path.clone()));
                }
            }
        }
        items.push(json!({"kind":"CHANGED_FACT","id":id,"before":before,"after":after,"beforeAuthority":"RETAINED_BASELINE","afterAuthority":"CURRENT_SOURCE_CHECK"}));
    }
    let current = checked.sources();
    for source in before_sources.values().chain(current.values()) {
        if changed_files.contains(&(source.service.clone(), source.file.clone())) {
            source_ids.insert(source.id.clone());
        }
    }
    for id in source_ids {
        let before = before_sources.get(&id);
        let after = current.get(&id);
        items.push(json!({"kind":"SOURCE_CHANGE","id":id,"before":before,"after":after,"beforeAvailable":before.is_some(),"afterAvailable":after.is_some(),"drill":after.map(|s|json!({"command":"docs context","service":s.service,"source":s.id}))}));
    }
    let involved_services: BTreeSet<_> = changed_files
        .iter()
        .map(|(service, _)| service.as_str())
        .collect();
    for source in current
        .values()
        .filter(|s| involved_services.contains(s.service.as_str()))
    {
        items.push(json!({"kind":"SUPPORTING_CONTEXT_REFERENCE","id":source.id,"service":source.service,"file":source.file,"startLine":source.start_line,"endLine":source.end_line,"authority":source.authority,"drill":{"command":"docs context","service":source.service,"source":source.id}}));
    }
    for (service, file) in changed_files {
        items.push(json!({"kind":"CHANGED_SCOPE_FILE","id":format!("{service}/{file}"),"service":service,"file":file,"requiredAction":"REVIEW_FILE_AND_ITS_HELPER_IMPORT_CONFIGURATION_CONTEXT"}));
    }
    items.push(json!({"kind":"COVERAGE","id":"coverage","unresolved":checked.unresolved,"catalogueChanges":report["catalogueChanges"],"linkChanges":report["linkChanges"],"services":checked.services.iter().map(|(id,e)|(id,json!({"coverage":e.coverage,"boundaries":e.boundaries}))).collect::<BTreeMap<_,_>>(),"scope":"Recorded claims plus conservative selected source scope; external reads require their own registered scope"}));
    page(
        &super::digest(&(&bundle, &snapshot, &checked.context_digest, &args.fragment))?,
        items,
        args.cursor.as_deref(),
        args.limit as usize,
        json!({"reportSchema":"codeclew-docs-changes/1.0","snapshot":snapshot,"authority":"PINNED_SNAPSHOT_NOT_REVERIFIED","baselineBundle":bundle,"contextDigest":checked.context_digest,"status":report["status"],"requiredAction":"Review affected claims before supplying a new narrative; this command does not publish"}),
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

    #[test]
    fn check_cursor_binds_report_identity_and_rejects_malformed_input() {
        let report_id = format!("sha256:{}", "a".repeat(64));
        let cursor = check_cursor(&report_id, 7);
        assert_eq!(
            parse_check_cursor(&cursor).unwrap(),
            (report_id.as_str(), 7)
        );
        for malformed in [
            "7",
            "check-v1|sha256:bad|7",
            "check-v1|sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa|x",
            "check-v1|sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa|7|extra",
        ] {
            assert!(parse_check_cursor(malformed).is_err(), "{malformed}");
        }
    }

    #[test]
    fn check_page_uses_identity_bound_cursor_for_followups() {
        let binding = CheckReportBinding {
            schema: CHECK_REPORT_SCHEMA.into(),
            report_id: format!("sha256:{}", "b".repeat(64)),
            snapshot: "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc/12"
                .into(),
            input_digest: "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
                .into(),
            context_digest:
                "sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee".into(),
            service_selection: vec!["service-a".into()],
            freshness: json!({"status":"STALE","affected":(0..100)
                .map(|id| json!({"fragment":id,"detail":"x".repeat(1024)}))
                .collect::<Vec<_>>(),"unaffected":[]}),
            output_binding:
                "sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff".into(),
        };
        let rows = (0..4).map(|id| json!({"id": id})).collect::<Vec<_>>();
        let first = check_page(&binding, rows.clone(), None, 2).unwrap();
        assert_eq!(first["reportSchema"], "codeclew-documentation-check/1.0");
        let cursor = first["nextCursor"].as_str().unwrap();
        assert!(cursor.starts_with("check-v1|sha256:"));
        let second = check_page(&binding, rows, Some(cursor), 2).unwrap();
        assert_eq!(second["items"][0]["id"], 2);

        // A large freshness report must fit across pages without copying its
        // affected list into the small per-page metadata budget.
        let rows = check_rows(&json!({"freshness":binding.freshness}));
        let mut cursor = None;
        let mut affected = Vec::new();
        loop {
            let result = check_page(&binding, rows.clone(), cursor.as_deref(), 100).unwrap();
            assert_eq!(result["freshness"], json!({"status":"STALE"}));
            assert!(crate::documentation::bytes(&result).unwrap().len() < 64 * 1024);
            assert!(result["omitted"].as_array().unwrap().is_empty());
            affected.extend(
                result["items"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|row| {
                        row["id"]
                            .as_str()
                            .is_some_and(|id| id.starts_with("affected/"))
                    })
                    .map(|row| row["record"].clone()),
            );
            cursor = result["nextCursor"].as_str().map(str::to_owned);
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(json!(affected), binding.freshness["affected"]);
    }

    #[test]
    fn check_dispatch_reloads_saved_snapshot_after_latest_and_declaration_changes() {
        use std::fs;

        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("docs");
        Repository::init(&root, "Synthetic docs").unwrap();
        let repo = Repository::open(&root).unwrap();
        let observation = Observation {
            id: "obs-a".into(),
            kind: "ENTRYPOINT".into(),
            service: "svc-a".into(),
            symbol: "pkg.A.run".into(),
            normalized: json!({"name":"run"}),
            digest: "sha256:1111111111111111111111111111111111111111111111111111111111111111"
                .into(),
            source_ids: Vec::new(),
        };
        let mut observations = BTreeMap::new();
        observations.insert(observation.id.clone(), observation.clone());
        let evidence = ServiceEvidence {
            schema: "codeclew-documentation-service-evidence/1.0".into(),
            service: "svc-a".into(),
            revision: "rev-a".into(),
            service_digest:
                "sha256:2222222222222222222222222222222222222222222222222222222222222222".into(),
            extractor: EXTRACTOR.into(),
            runtime_mode: "SOURCE".into(),
            coverage: "COMPLETE".into(),
            boundaries: Vec::new(),
            entrypoints: Vec::new(),
            observations: observations.clone(),
            sources: BTreeMap::new(),
            contracts: BTreeMap::new(),
        };
        let mut services = BTreeMap::new();
        services.insert("svc-a".into(), evidence);
        let checked = check::Check {
            schema: "codeclew-documentation-check/1.0".into(),
            source_inputs: None,
            composition: None,
            input_digest: "sha256:3333333333333333333333333333333333333333333333333333333333333333"
                .into(),
            context_digest:
                "sha256:4444444444444444444444444444444444444444444444444444444444444444".into(),
            services,
            unresolved: BTreeMap::new(),
            interactions: BTreeMap::new(),
            scenarios: BTreeMap::new(),
            dependencies: observations,
        };
        let snapshot = checked.save_snapshot(&repo).unwrap();
        let selection = BTreeSet::from(["svc-a".to_string()]);
        let mut value = checked.summary();
        value["freshness"] = json!({"status":"CURRENT","reasons":[],"affected":[]});
        value["snapshot"] = json!(snapshot.clone());
        let output_binding = check_output_binding(&checked, &selection, &snapshot, &value).unwrap();
        let binding = CheckReportBinding {
            schema: CHECK_REPORT_SCHEMA.into(),
            report_id: crate::documentation::digest(&(
                snapshot.clone(),
                selection.iter().cloned().collect::<Vec<_>>(),
                output_binding.clone(),
            ))
            .unwrap(),
            snapshot: snapshot.clone(),
            input_digest: checked.input_digest.clone(),
            context_digest: checked.context_digest.clone(),
            service_selection: selection.iter().cloned().collect(),
            freshness: value["freshness"].clone(),
            output_binding,
        };
        save_check_report(&repo, &binding).unwrap();
        let first = check_page(&binding, check_rows(&value), None, 1).unwrap();
        let cursor = first["nextCursor"].as_str().unwrap().to_string();
        let expected = check_page(&binding, check_rows(&value), Some(&cursor), 1).unwrap();

        repo.atomic(
            ".codeclew/cache/latest-check.json",
            b"malformed latest pointer",
        )
        .unwrap();
        repo.atomic("catalog/services/changed.yaml", b"changed declaration")
            .unwrap();
        let missing_debug = root.join("debug-output-does-not-exist");
        let followup = run(Command::Check(CheckArgs {
            page: ListArgs {
                root: root.clone(),
                cursor: Some(cursor.clone()),
                limit: 1,
            },
            services: vec!["svc-a".into()],
            debug_output: Some(missing_debug.clone()),
        }))
        .unwrap();
        assert_eq!(followup["items"], expected["items"]);
        assert_eq!(followup["freshness"], expected["freshness"]);
        assert_eq!(followup["snapshot"], expected["snapshot"]);

        assert!(
            run(Command::Check(CheckArgs {
                page: ListArgs {
                    root: root.clone(),
                    cursor: Some(cursor.clone()),
                    limit: 1,
                },
                services: vec!["svc-b".into()],
                debug_output: None,
            }))
            .is_err()
        );

        let mut corrupt = binding.clone();
        corrupt.output_binding =
            "sha256:9999999999999999999999999999999999999999999999999999999999999999".into();
        repo.atomic(
            &format!(
                ".codeclew/cache/check-reports/{}.json",
                binding.report_id.trim_start_matches("sha256:")
            ),
            &crate::documentation::bytes(&corrupt).unwrap(),
        )
        .unwrap();
        assert!(
            run(Command::Check(CheckArgs {
                page: ListArgs {
                    root: root.clone(),
                    cursor: Some(cursor.clone()),
                    limit: 1,
                },
                services: vec!["svc-a".into()],
                debug_output: None,
            }))
            .is_err()
        );
        save_check_report(&repo, &binding).unwrap();

        let digest = binding.snapshot.rsplit_once('/').unwrap().0;
        let object = repo
            .root
            .join(super::super::cache::OBJECT_ROOT)
            .join(digest)
            .join("object.json");
        fs::remove_file(object).unwrap();
        assert!(
            run(Command::Check(CheckArgs {
                page: ListArgs {
                    root,
                    cursor: Some(cursor),
                    limit: 1,
                },
                services: vec!["svc-a".into()],
                debug_output: None,
            }))
            .is_err()
        );
        assert!(missing_debug.ends_with("debug-output-does-not-exist"));
    }
}
