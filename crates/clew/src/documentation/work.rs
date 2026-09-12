//! Immutable evidence work and bounded, recorded reads for external authors.
use super::{
    bindings, bytes,
    check::{self, Check},
    cli::{self, ContextArgs, ContextFormat},
    digest, invalid, io_error,
    model::*,
    store::{self, Repository},
};
use crate::error::{ClewError, ErrorCode};
use clap::{Args, Subcommand};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::PathBuf,
};

#[derive(Debug, Subcommand)]
pub enum Command {
    Prepare {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        subject: String,
        #[arg(long)]
        input: PathBuf,
    },
    Run {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        work: String,
        #[arg(long)]
        config: Option<PathBuf>,
    },
    Status {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        work: String,
        #[arg(long)]
        cursor: Option<String>,
        #[arg(long,default_value_t=20,value_parser=clap::value_parser!(u32).range(1..=100))]
        limit: u32,
    },
    Cancel {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        work: String,
    },
    Read(ReadArgs),
    Expand(ReadArgs),
}
#[derive(Debug, Args)]
pub struct ReadArgs {
    #[arg(long)]
    pub root: PathBuf,
    #[arg(long)]
    pub work: String,
    #[arg(long)]
    pub input: PathBuf,
}
fn default_limit() -> u32 {
    20
}
fn default_bytes() -> usize {
    40 * 1024
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    pub schema: String,
    pub audience: String,
    #[serde(default)]
    pub entrypoint: Option<String>,
    #[serde(default = "default_limit")]
    pub max_items: u32,
    #[serde(default = "default_bytes")]
    pub max_bytes: usize,
    #[serde(default)]
    pub external_inputs: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Selection {
    #[serde(default)]
    pub references: Vec<String>,
    #[serde(default)]
    pub symbols: Vec<String>,
    #[serde(default)]
    pub query: Option<Query>,
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub untracked_reads: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Query {
    pub kind: String,
    #[serde(default)]
    pub symbol_contains: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Handle {
    pub kind: String,
    pub id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Work {
    pub schema: String,
    pub id: String,
    pub subject: String,
    pub request: Request,
    pub checked: Check,
    pub retained: Option<Narrative>,
    pub external_inputs: BTreeMap<String, Value>,
    pub handles: BTreeMap<String, Handle>,
    pub influence: BTreeMap<String, String>,
    pub obligations: Vec<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub review_reasons: Vec<Value>,
}
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadState {
    pub work: String,
    pub receipts: BTreeMap<String, ReadReceipt>,
    pub untracked_reads: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadReceipt {
    pub selection: Selection,
    pub result_digest: String,
    pub supplied: Vec<String>,
    pub membership_digest: String,
    pub omitted: Vec<Value>,
    pub next_cursor: Option<String>,
}
fn directory(id: &str) -> Result<String, ClewError> {
    if id.len() != 64 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(invalid("invalid work identity"));
    }
    Ok(format!(".codeclew/work/{id}"))
}
pub fn load(repo: &Repository, id: &str) -> Result<Work, ClewError> {
    let mut work: Work = store::read(
        &repo.path(&format!("{}/work.json", directory(id)?))?,
        64 * 1024 * 1024,
    )?;
    let recorded = work.id.clone();
    work.id.clear();
    let expected = digest(&work)?[7..].to_owned();
    work.id = recorded;
    if work.id != id || expected != id || work.schema != "codeclew-documentation-work/1.0" {
        return Err(invalid("work evidence digest or schema is invalid"));
    }
    Ok(work)
}
pub fn read_state(repo: &Repository, id: &str) -> Result<ReadState, ClewError> {
    let path = repo.path(&format!("{}/reads.json", directory(id)?))?;
    if path.exists() {
        store::read(&path, 16 * 1024 * 1024)
    } else {
        Ok(ReadState {
            work: id.into(),
            ..Default::default()
        })
    }
}
pub fn run(command: Command) -> Result<Value, ClewError> {
    match command {
        Command::Prepare {
            root,
            subject,
            input,
        } => prepare(
            &Repository::open(&root)?,
            subject,
            store::read(&input, store::MAX_RECORD)?,
        ),
        Command::Run { root, work, config } => {
            super::agent_jobs::run(&Repository::open(&root)?, &work, config.as_deref())
        }
        Command::Status {
            root,
            work,
            cursor,
            limit,
        } => super::agent_jobs::status(
            &Repository::open(&root)?,
            &work,
            cursor.as_deref(),
            limit as usize,
        ),
        Command::Cancel { root, work } => {
            super::agent_jobs::cancel(&Repository::open(&root)?, &work)
        }
        Command::Read(args) | Command::Expand(args) => read(
            &Repository::open(&args.root)?,
            &args.work,
            store::read(&args.input, store::MAX_RECORD)?,
        ),
    }
}

pub fn prepare(repo: &Repository, subject: String, request: Request) -> Result<Value, ClewError> {
    if request.schema != "codeclew-documentation-work-request/1.0"
        || request.audience.trim().is_empty()
        || request.audience.len() > 512
        || !(1..=100).contains(&request.max_items)
        || !(2048..=48 * 1024).contains(&request.max_bytes)
    {
        return Err(invalid(
            "work requires an audience, 1..100 items and 2048..49152 bytes",
        ));
    }
    let (kind, id) = subject
        .split_once(':')
        .ok_or_else(|| invalid("work subject must be service:ID or scenario:ID"))?;
    let selected = match kind {
        "service" if repo.services()?.contains_key(id) => BTreeSet::from([id.to_owned()]),
        "scenario"
            if repo.scenarios()?.contains_key(id)
                && (request.entrypoint.is_none()
                    || (request.entrypoint.as_deref() == Some(super::processes::OVERVIEW)
                        && repo.scenarios()?[id].process.is_some())) =>
        {
            BTreeSet::new()
        }
        _ => {
            return Err(invalid(
                "unknown work subject or unsupported scenario entrypoint",
            ));
        }
    };
    let checked = check::run_selected(repo, &selected)?;
    checked.save(repo)?;
    let baseline = bindings::baseline(repo)?;
    let retained = baseline
        .as_ref()
        .and_then(|(_, b)| b.narratives.get(&subject).cloned());
    let changes = bindings::freshness(baseline.as_ref().map(|(_, b)| b), &checked);
    let mut review_reasons: Vec<Value> = changes["affected"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|v| v["subject"] == subject)
        .cloned()
        .collect();
    review_reasons.extend(
        changes["catalogueChanges"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|v| kind == "scenario" || v["service"] == id)
            .cloned(),
    );
    if baseline.is_none() {
        review_reasons.push(json!({"reason":"MISSING_BASELINE"}));
    }

    let mut handles = BTreeMap::new();
    for (prefix, kind, ids) in [
        (
            "e",
            "ENTRYPOINT",
            checked
                .services
                .values()
                .flat_map(|s| s.entrypoints.iter().map(|e| e.id.clone()))
                .collect::<BTreeSet<_>>(),
        ),
        (
            "d",
            "DEPENDENCY",
            checked.dependencies.keys().cloned().collect(),
        ),
        ("s", "SOURCE", checked.sources().keys().cloned().collect()),
    ] {
        for (index, id) in ids.into_iter().enumerate() {
            handles.insert(
                format!("{prefix}{}", index + 1),
                Handle {
                    kind: kind.into(),
                    id,
                },
            );
        }
    }
    if kind == "service" {
        for (index, id) in super::sections::ids().enumerate() {
            handles.insert(
                format!("section{}", index + 1),
                Handle {
                    kind: "SECTION".into(),
                    id,
                },
            );
        }
    }
    if kind == "service" {
        for (i, note) in super::notes::for_service(&checked, id).enumerate() {
            handles.insert(
                format!("note{}", i + 1),
                Handle {
                    kind: "NOTE".into(),
                    id: note.id.clone(),
                },
            );
        }
    }
    let component_scope = checked
        .scenarios
        .get(id)
        .map(|s| bindings::expand_dependencies(&s.dependency_ids, &checked))
        .transpose()?
        .unwrap_or_default();
    let mut influence: BTreeMap<String, String> = checked
        .dependencies
        .iter()
        .filter(|(id, o)| {
            o.kind != "PROCESS_COMPONENT" || (kind == "scenario" && component_scope.contains(*id))
        })
        .map(|(id, o)| (id.clone(), o.digest.clone()))
        .collect();
    let mut obligations = Vec::new();
    for (service, failure) in &checked.unresolved {
        if failure["reason"] != "SERVICE_NOT_SELECTED" {
            obligations.push(json!({"kind":"MISSING_SERVICE","service":service,"detail":failure}));
        }
    }
    for (service, evidence) in &checked.services {
        for boundary in &evidence.boundaries {
            obligations
                .push(json!({"kind":"EVIDENCE_BOUNDARY","service":service,"detail":boundary}));
        }
        if evidence.extractor == SOURCE_EXTRACTOR {
            obligations.push(json!({"kind":"UNRESOLVED_CALL_AUTHORITY","service":service,"detail":"Syntax evidence does not establish runtime dispatch or configuration. Expand known owners and helpers; retain explicit gaps for unresolved targets."}));
        }
    }
    let external_inputs = capture_inputs(repo, &request)?;
    influence.insert(
        "documentation:external-inputs".into(),
        digest(&external_inputs)?,
    );
    for (path, record) in &external_inputs {
        if record["status"] != "CAPTURED" && !(path == "notes" && record["status"] == "ABSENT") {
            obligations.push(json!({"kind":"MISSING_EXTERNAL_INPUT","path":path,"detail":record}));
        }
    }
    let mut work = Work {
        schema: "codeclew-documentation-work/1.0".into(),
        id: String::new(),
        subject,
        request,
        checked,
        retained,
        external_inputs,
        handles,
        influence,
        obligations,
        review_reasons,
    };
    work.id = digest(&work)?[7..].into();
    // Validate selection before committing an unusable work object.
    rows(&work, &Selection::default())?;
    let encoded = bytes(&work)?;
    if encoded.len() > 64 * 1024 * 1024 {
        return Err(ClewError::new(
            ErrorCode::SliceBudgetExceeded,
            "work capture exceeds 64 MiB; narrow the service scope",
        ));
    }
    {
        let _lock = repo.lock()?;
        if capture_inputs(repo, &work.request)? != work.external_inputs {
            return Err(invalid(
                "human or external inputs changed during work preparation",
            ));
        }
        if repo.input_digest()? != work.checked.input_digest {
            return Err(invalid(
                "documentation declarations changed during work preparation",
            ));
        }
        let path = format!("{}/work.json", directory(&work.id)?);
        if repo.path(&path)?.exists() {
            load(repo, &work.id)?;
        } else {
            repo.atomic(&path, &encoded)?;
        }
    }
    read(repo, &work.id, Selection::default())
}

// Only explicitly admitted root-relative files and the protected notes tree are read.
// The tree membership itself is captured, including absent notes and failed reads.
pub fn capture_inputs(
    repo: &Repository,
    request: &Request,
) -> Result<BTreeMap<String, Value>, ClewError> {
    if request.external_inputs.len() > 64 {
        return Err(invalid("select at most 64 external inputs"));
    }
    let mut paths: BTreeSet<String> = request.external_inputs.iter().cloned().collect();
    let mut pending = vec!["notes".to_owned()];
    let mut traversed = 0;
    while let Some(relative) = pending.pop() {
        traversed += 1;
        if traversed > 128 {
            return Err(invalid(
                "protected notes tree exceeds 128 entries; narrow the documentation root",
            ));
        }
        let path = repo.path(&relative)?;
        if !path.exists() {
            paths.insert(relative);
            continue;
        }
        let metadata = fs::symlink_metadata(&path).map_err(io_error)?;
        if metadata.is_dir() {
            for child in fs::read_dir(path).map_err(io_error)? {
                let name = child
                    .map_err(io_error)?
                    .file_name()
                    .into_string()
                    .map_err(|_| invalid("note path is not UTF-8"))?;
                pending.push(format!("{relative}/{name}"));
            }
        } else {
            paths.insert(relative);
        }
    }
    let mut out = BTreeMap::new();
    for relative in paths {
        let path = repo.path(&relative)?;
        let value = match fs::metadata(&path) {
            Ok(meta) if meta.is_file() && meta.len() <= 256 * 1024 => {
                match fs::read_to_string(&path) {
                    Ok(text) if text.len() <= 256 * 1024 => {
                        json!({"status":"CAPTURED","authority":"HUMAN_OR_IMPORTED_UNVERIFIED","digest":digest(&text)?,"text":text})
                    }
                    _ => json!({"status":"UNAVAILABLE","reason":"NOT_BOUNDED_UTF8"}),
                }
            }
            Ok(_) => json!({"status":"UNAVAILABLE","reason":"NOT_BOUNDED_FILE"}),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                json!({"status":"ABSENT"})
            }
            Err(_) => json!({"status":"UNAVAILABLE","reason":"READ_FAILED"}),
        };
        out.insert(relative, value);
    }
    Ok(out)
}

fn rows(work: &Work, selection: &Selection) -> Result<Vec<Value>, ClewError> {
    if selection.references.len() > 8
        || selection.symbols.len() > 8
        || selection
            .query
            .as_ref()
            .is_some_and(|q| q.kind.len() > 100 || q.symbol_contains.len() > 512)
        || (!selection.references.is_empty() && !selection.symbols.is_empty())
        || (selection.query.is_some()
            && (!selection.references.is_empty() || !selection.symbols.is_empty()))
    {
        return Err(invalid(
            "choose up to eight references, eight symbols, or one bounded query",
        ));
    }
    let (kind, id) = work
        .subject
        .split_once(':')
        .ok_or_else(|| invalid("invalid stored work subject"))?;
    let section_selection = kind == "service"
        && ((selection.references.is_empty()
            && selection.symbols.is_empty()
            && selection.query.is_none()
            && work
                .request
                .entrypoint
                .as_deref()
                .is_some_and(super::sections::contains))
            || selection
                .references
                .iter()
                .any(|r| work.handles.get(r).is_some_and(|h| h.kind == "SECTION")));
    let note_selection = kind == "service"
        && ((selection.references.is_empty()
            && selection.symbols.is_empty()
            && selection.query.is_none()
            && work
                .request
                .entrypoint
                .as_deref()
                .is_some_and(super::notes::is_root))
            || selection
                .references
                .iter()
                .any(|r| work.handles.get(r).is_some_and(|h| h.kind == "NOTE")));
    let mut items = if note_selection {
        if !selection.symbols.is_empty()
            || selection.query.is_some()
            || selection.references.len() > 1
        {
            return Err(invalid("select a note separately from source expansions"));
        }
        let mut rows: Vec<_> = super::notes::for_service(&work.checked, id)
            .map(|d| json!({"kind":"NOTE","id":d.id,"record":d}))
            .collect();
        rows.extend(
            work.checked
                .dependencies
                .values()
                .filter(|d| d.service == id && d.kind != "NOTE_ASSOCIATION")
                .map(|d| json!({"kind":"DEPENDENCY","id":d.id,"record":d})),
        );
        rows
    } else if section_selection {
        if !selection.symbols.is_empty()
            || selection.query.is_some()
            || selection.references.len() > 1
        {
            return Err(invalid(
                "select a section separately from source expansions",
            ));
        }
        let mut rows: Vec<Value> = super::sections::records(id, work.retained.as_ref())
            .into_iter()
            .map(|r| json!({"kind":"SECTION","id":r["id"],"record":r}))
            .collect();
        rows.push(json!({"kind":"BOUNDARY_INVENTORY","id":format!("inventory:{id}"),"record":super::sections::inventory(id,&work.checked)}));
        rows.extend(
            work.checked
                .dependencies
                .values()
                .filter(|d| d.service == id || d.kind == "DOMAIN_ENTITY")
                .map(|d| json!({"kind":"DEPENDENCY","id":d.id,"record":d})),
        );
        rows
    } else if let Some(query) = &selection.query {
        if query.kind.trim().is_empty() {
            return Err(invalid(
                "query kind is required; use * for every dependency kind",
            ));
        }
        work.checked
            .dependencies
            .values()
            .filter(|d| {
                (query.kind == "*" || d.kind == query.kind)
                    && d.symbol.contains(&query.symbol_contains)
            })
            .map(|d| json!({"kind":"DEPENDENCY","id":d.id,"record":d}))
            .collect()
    } else {
        let mut args = ContextArgs {
            root: PathBuf::new(),
            service: (kind == "service").then(|| id.into()),
            scenario: (kind == "scenario").then(|| id.into()),
            entrypoint: None,
            symbols: selection.symbols.clone(),
            source_ids: Vec::new(),
            dependency_ids: Vec::new(),
            format: ContextFormat::Raw,
            refresh: false,
            cursor: None,
            limit: 100,
        };
        for reference in &selection.references {
            let handle = work
                .handles
                .get(reference)
                .ok_or_else(|| invalid("unknown work reference"))?;
            match handle.kind.as_str() {
                "ENTRYPOINT" => {
                    if args.entrypoint.replace(handle.id.clone()).is_some() {
                        return Err(invalid("select one entrypoint"));
                    }
                }
                "DEPENDENCY" => args.dependency_ids.push(handle.id.clone()),
                "SOURCE" => args.source_ids.push(handle.id.clone()),
                _ => return Err(invalid("invalid work reference kind")),
            }
        }
        if selection.references.is_empty() && selection.symbols.is_empty() {
            args.entrypoint = work.request.entrypoint.clone();
        }
        if args.entrypoint.is_some()
            && (!args.source_ids.is_empty() || !args.dependency_ids.is_empty())
        {
            return Err(invalid(
                "entrypoint references cannot be mixed with other selections",
            ));
        }
        if kind == "scenario" && (!selection.references.is_empty() || !selection.symbols.is_empty())
        {
            let sources = work.checked.sources();
            let mut records = Vec::new();
            if !selection.symbols.is_empty() || args.entrypoint.is_some() {
                return Err(invalid(
                    "scenario expansions require dependency or source references",
                ));
            }
            for id in &args.dependency_ids {
                records.push(
                    json!({"kind":"DEPENDENCY","id":id,"record":work.checked.dependencies[id]}),
                );
            }
            for id in &args.source_ids {
                records.push(json!({"kind":"SOURCE","id":id,"record":sources[id]}));
            }
            records
        } else if kind == "service" && !work.checked.services.contains_key(id) {
            Vec::new()
        } else {
            cli::context_items(&work.checked, &args, work.retained.as_ref())?
        }
    };
    if selection.query.is_none() && selection.references.is_empty() && selection.symbols.is_empty()
    {
        if kind == "service" && !section_selection {
            for row in super::sections::records(id, work.retained.as_ref()) {
                items.push(json!({"kind":"SECTION","id":row["id"],"record":row}));
            }
        }
        for (index, record) in work.review_reasons.iter().enumerate() {
            items.push(
                json!({"kind":"REVIEW_REASON","id":format!("review-{index}"),"record":record}),
            );
        }
        for (path, record) in &work.external_inputs {
            items.push(json!({"kind":"EXTERNAL_INPUT","id":path,"record":record}));
        }
        for (index, obligation) in work.obligations.iter().enumerate() {
            items.push(json!({"kind":"OBLIGATION","id":format!("obligation-{}",index+1),"record":obligation}));
        }
    }
    let reverse: BTreeMap<_, _> = work
        .handles
        .iter()
        .map(|(key, h)| ((h.kind.as_str(), h.id.as_str()), key.as_str()))
        .collect();
    for item in &mut items {
        if let Some(reference) = reverse.get(&(
            item["kind"].as_str().unwrap_or(""),
            item["id"].as_str().unwrap_or(""),
        )) {
            item["reference"] = json!(reference);
        }
        for field in ["sourceIds", "dependencyIds"] {
            if let Some(ids) = item["record"][field].as_array() {
                let references: Vec<_> = ids
                    .iter()
                    .filter_map(|id| {
                        reverse
                            .get(&(
                                if field == "sourceIds" {
                                    "SOURCE"
                                } else {
                                    "DEPENDENCY"
                                },
                                id.as_str()?,
                            ))
                            .copied()
                    })
                    .collect();
                item[if field == "sourceIds" {
                    "sourceReferences"
                } else {
                    "dependencyReferences"
                }] = json!(references);
            }
        }
    }
    Ok(items)
}

pub fn read(repo: &Repository, id: &str, selection: Selection) -> Result<Value, ClewError> {
    let work = load(repo, id)?;
    let items = rows(&work, &selection)?;
    let membership: Vec<_> = items.iter().map(|i| json!([i["kind"], i["id"]])).collect();
    let membership_digest = digest(&membership)?;
    let mut binding_selection = selection.clone();
    binding_selection.cursor = None;
    binding_selection.untracked_reads = false;
    let binding = digest(&(id, &binding_selection))?;
    let prefix = &binding[7..];
    let start = match selection.cursor.as_deref() {
        None => 0,
        Some(cursor) => {
            let (owner, index) = cursor
                .split_once(':')
                .ok_or_else(|| invalid("invalid work cursor"))?;
            if owner != prefix {
                return Err(invalid("work cursor belongs to another work or selection"));
            }
            index
                .parse::<usize>()
                .map_err(|_| invalid("invalid work cursor offset"))?
        }
    };
    if start > items.len() {
        return Err(invalid("work cursor is out of range"));
    }
    let mut output = json!({"schema":"codeclew-documentation-work-page/1.0","work":id,"subject":work.subject,"audience":work.request.audience,
        "contextDigest":work.checked.context_digest,"inputDigest":work.checked.input_digest,"authority":"IMMUTABLE_WORK_CAPTURE_NOT_REVERIFIED",
        "influenceCoverage":"RECORDED_READS_ONLY_EXECUTION_NOT_ATTESTED","membershipDigest":membership_digest,
        "total":items.len(),"items":[],"omitted":[],"nextCursor":null});
    let mut supplied = Vec::new();
    let mut omitted = Vec::new();
    let mut out = Vec::new();
    let mut consumed = start;
    for item in items
        .into_iter()
        .skip(start)
        .take(work.request.max_items as usize)
    {
        let mut candidate = output.clone();
        let mut trial = out.clone();
        trial.push(item.clone());
        candidate["items"] = json!(trial);
        // Reserve space for cursor, receipt digest, and influence status.
        if bytes(&candidate)?.len() + 512 > work.request.max_bytes {
            if !out.is_empty() || !omitted.is_empty() {
                break;
            }
            omitted.push(json!({"index":consumed,"kind":item["kind"],"id":item["id"],"reference":item["reference"],"reason":"ITEM_EXCEEDS_WORK_BYTE_BUDGET"}));
            output["omitted"] = json!(omitted);
            consumed += 1;
            break;
        }
        if let Some(reference) = item["reference"].as_str() {
            supplied.push(reference.to_owned());
        }
        out.push(item);
        consumed += 1;
    }
    output["items"] = json!(out);
    if consumed < output["total"].as_u64().unwrap() as usize {
        output["nextCursor"] = json!(format!("{prefix}:{consumed}"));
    }
    let _lock = repo.lock()?;
    let mut state = read_state(repo, id)?;
    if state.work != id {
        return Err(invalid("read ledger belongs to another work"));
    }
    state.untracked_reads |= selection.untracked_reads;
    if state.untracked_reads {
        output["influenceCoverage"] = json!("INCOMPLETE_UNTRACKED_READS");
    }
    let result_digest = digest(&output)?;
    let receipt = ReadReceipt {
        selection: selection.clone(),
        result_digest: result_digest.clone(),
        supplied,
        membership_digest,
        omitted,
        next_cursor: output["nextCursor"].as_str().map(str::to_owned),
    };
    output["receiptDigest"] = json!(result_digest);
    if bytes(&output)?.len() + 1 > work.request.max_bytes {
        return Err(invalid("work page metadata exceeds requested byte budget"));
    }
    state.receipts.insert(digest(&receipt)?, receipt);
    let encoded = bytes(&state)?;
    if encoded.len() > 16 * 1024 * 1024 {
        return Err(invalid(
            "work read ledger exceeds its bound; prepare narrower work",
        ));
    }
    repo.atomic(&format!("{}/reads.json", directory(id)?), &encoded)?;
    Ok(output)
}

/// All initially required facts, old content and human inputs must be supplied.
/// Omitted records are an evidence-budget problem, never a model-reasoning problem.
pub fn initial_context_complete(state: &ReadState) -> bool {
    let mut cursor: Option<String> = None;
    for _ in 0..=state.receipts.len() {
        let Some(receipt) = state.receipts.values().find(|r| {
            r.selection.references.is_empty()
                && r.selection.symbols.is_empty()
                && r.selection.query.is_none()
                && r.selection.cursor == cursor
                && r.omitted.is_empty()
        }) else {
            return false;
        };
        if receipt.next_cursor.is_none() {
            return true;
        }
        cursor = receipt.next_cursor.clone();
    }
    false
}
