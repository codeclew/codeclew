//! Immutable evidence work and bounded, recorded reads for external authors.
use super::{
    bindings, bytes,
    check::Check,
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

pub use super::work_parts::{SourcePartReceipt, SourcePartRequest, read_part};
pub(super) use super::work_parts::{
    completed_source_references, initial_context_complete_with_parts,
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
        /// Select saved evidence; defaults to the latest saved check, never captures.
        #[arg(long)]
        snapshot: Option<String>,
        /// Language of authored documentation prose, independent of source language.
        #[arg(long, value_parser = ["en", "ru"])]
        language: Option<String>,
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
    ReadPart(ReadPartArgs),
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
#[derive(Debug, Args)]
pub struct ReadPartArgs {
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub documentation_language: Option<String>,
    #[serde(default)]
    pub entrypoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_profile: Option<String>,
    #[serde(default = "default_limit")]
    pub max_items: u32,
    #[serde(default = "default_bytes")]
    pub max_bytes: usize,
    #[serde(default)]
    pub external_inputs: Vec<String>,
}
impl Request {
    pub fn documentation_language(&self) -> &str {
        self.documentation_language.as_deref().unwrap_or("en")
    }

    fn normalize_documentation_language(&mut self) -> Result<(), ClewError> {
        self.validate_documentation_language()?;
        self.documentation_language
            .get_or_insert_with(|| "en".into());
        Ok(())
    }

    fn with_language_flag(mut self, language: Option<String>) -> Result<Self, ClewError> {
        if let Some(language) = language {
            if self
                .documentation_language
                .as_ref()
                .is_some_and(|input| input != &language)
            {
                return Err(invalid(
                    "--language conflicts with input documentationLanguage",
                ));
            }
            self.documentation_language = Some(language);
        }
        self.validate_documentation_language()?;
        Ok(self)
    }

    pub(super) fn validate_documentation_language(&self) -> Result<(), ClewError> {
        if matches!(
            self.documentation_language.as_deref(),
            None | Some("en" | "ru")
        ) {
            Ok(())
        } else {
            Err(invalid("documentationLanguage must be en or ru"))
        }
    }
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<String>,
    pub retained: Option<Narrative>,
    pub external_inputs: BTreeMap<String, Value>,
    pub handles: BTreeMap<String, Handle>,
    pub influence: BTreeMap<String, String>,
    pub obligations: Vec<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub review_reasons: Vec<Value>,
}

const SECTION_ORIENTATION_PROFILE: &str = "section-orientation-v1";
const WORK_SCHEMA: &str = "codeclew-documentation-work/1.0";
const WORK_MANIFEST_SCHEMA: &str = "codeclew-documentation-work-manifest/1.0";

/// The persisted work record keeps the immutable check in the content-addressed
/// cache instead of duplicating its (potentially very large) hydrated form.
/// `Work` remains the public, hydrated runtime representation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredWork {
    schema: String,
    id: String,
    subject: String,
    request: Request,
    snapshot: String,
    retained: Option<Narrative>,
    external_inputs: BTreeMap<String, Value>,
    handles: BTreeMap<String, Handle>,
    influence: BTreeMap<String, String>,
    obligations: Vec<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    review_reasons: Vec<Value>,
    evidence_snapshot: String,
}

impl StoredWork {
    fn from_runtime(work: &Work, evidence_snapshot: String) -> Self {
        Self {
            schema: WORK_MANIFEST_SCHEMA.into(),
            id: String::new(),
            subject: work.subject.clone(),
            request: work.request.clone(),
            snapshot: evidence_snapshot.clone(),
            retained: work.retained.clone(),
            external_inputs: work.external_inputs.clone(),
            handles: work.handles.clone(),
            influence: work.influence.clone(),
            obligations: work.obligations.clone(),
            review_reasons: work.review_reasons.clone(),
            evidence_snapshot,
        }
    }

    fn validate_identity(&self, id: &str) -> Result<(), ClewError> {
        if self.schema != WORK_MANIFEST_SCHEMA {
            return Err(invalid(
                "DOCS_REINDEX_REQUIRED: unsupported stored work schema; initialize a fresh documentation root, run docs check, and prepare new work",
            ));
        }
        if self.snapshot.is_empty() || self.snapshot != self.evidence_snapshot {
            return Err(invalid(
                "DOCS_REINDEX_REQUIRED: work requires an explicit snapshot matching its retained evidence; initialize a fresh documentation root, run docs check, and prepare new work",
            ));
        }
        let recorded = self.id.clone();
        let mut canonical = self.clone();
        canonical.id.clear();
        let expected = digest(&canonical)?[7..].to_owned();
        if recorded != id || expected != id {
            return Err(invalid("work evidence digest or schema is invalid"));
        }
        Ok(())
    }

    fn into_runtime(self, checked: Check) -> Work {
        Work {
            schema: WORK_SCHEMA.into(),
            id: self.id,
            subject: self.subject,
            request: self.request,
            checked,
            snapshot: Some(self.snapshot),
            retained: self.retained,
            external_inputs: self.external_inputs,
            handles: self.handles,
            influence: self.influence,
            obligations: self.obligations,
            review_reasons: self.review_reasons,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadState {
    pub work: String,
    pub receipts: BTreeMap<String, ReadReceipt>,
    pub untracked_reads: bool,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub source_part_receipts: BTreeMap<String, SourcePartReceipt>,
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
pub(super) fn directory(id: &str) -> Result<String, ClewError> {
    if id.len() != 64 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(invalid("invalid work identity"));
    }
    Ok(format!(".codeclew/work/{id}"))
}
fn load_stored(repo: &Repository, id: &str) -> Result<StoredWork, ClewError> {
    let path = repo.path(&format!("{}/work.json", directory(id)?))?;
    let stored: StoredWork = store::read(&path, 64 * 1024 * 1024).map_err(|error| invalid(format!(
        "DOCS_REINDEX_REQUIRED: unsupported saved work format ({}); initialize a fresh documentation root, run docs check, and prepare new work", error.message)))?;
    stored.validate_identity(id)?;
    validate_context_profile(&stored.subject, &stored.request)?;
    stored.request.validate_documentation_language()?;
    Ok(stored)
}

pub(super) fn load_influence(
    repo: &Repository,
    id: &str,
) -> Result<BTreeMap<String, String>, ClewError> {
    Ok(load_stored(repo, id)?.influence)
}

pub fn load(repo: &Repository, id: &str) -> Result<Work, ClewError> {
    let stored = super::progress::run("LOAD_WORK_RECORD", || load_stored(repo, id))?;
    let checked = super::progress::run("LOAD_RETAINED_SNAPSHOT", || {
        Check::load_snapshot(repo, &stored.evidence_snapshot)
    })?;
    Ok(stored.into_runtime(checked))
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
            snapshot,
            language,
        } => prepare_with_snapshot(
            &Repository::open(&root)?,
            subject,
            store::read::<Request>(&input, store::MAX_RECORD)?.with_language_flag(language)?,
            snapshot.as_deref(),
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
        Command::ReadPart(args) => read_part(
            &Repository::open(&args.root)?,
            &args.work,
            store::read(&args.input, store::MAX_RECORD)?,
        ),
    }
}

pub fn prepare(repo: &Repository, subject: String, request: Request) -> Result<Value, ClewError> {
    prepare_with_snapshot(repo, subject, request, None)
}

fn normalize_section_profile(subject: &str, request: &mut Request) {
    if request.context_profile.is_none()
        && subject.starts_with("service:")
        && request
            .entrypoint
            .as_deref()
            .is_some_and(super::sections::contains)
    {
        request.context_profile = Some(SECTION_ORIENTATION_PROFILE.into());
    }
}

fn validate_context_profile(subject: &str, request: &Request) -> Result<(), ClewError> {
    match request.context_profile.as_deref() {
        None => Ok(()),
        Some(SECTION_ORIENTATION_PROFILE)
            if subject.starts_with("service:")
                && request
                    .entrypoint
                    .as_deref()
                    .is_some_and(super::sections::contains) =>
        {
            Ok(())
        }
        Some(SECTION_ORIENTATION_PROFILE) => Err(invalid(
            "CONTEXT_PROFILE_INCOMPATIBLE: section-orientation-v1 requires one service section",
        )),
        Some("process-v1")
            if subject.starts_with("scenario:")
                && request.entrypoint.as_deref() == Some(super::processes::OVERVIEW) =>
        {
            Ok(())
        }
        Some("process-v1") => Err(invalid(
            "CONTEXT_PROFILE_INCOMPATIBLE: process-v1 requires a saved process overview",
        )),
        Some("declarations-v1")
            if subject
                .strip_prefix("service:")
                .is_some_and(|id| !id.is_empty())
                && request.entrypoint.as_deref() == Some("section-entities") =>
        {
            Ok(())
        }
        Some("declarations-v1") => Err(invalid(
            "CONTEXT_PROFILE_INCOMPATIBLE: declarations-v1 requires a service work request for section-entities",
        )),
        Some(profile) => Err(invalid(format!(
            "CONTEXT_PROFILE_UNSUPPORTED: unsupported immutable work context profile: {profile}"
        ))),
    }
}

/// Snapshot selection is explicit and never falls back to source acquisition.
pub fn prepare_with_snapshot(
    repo: &Repository,
    subject: String,
    mut request: Request,
    snapshot: Option<&str>,
) -> Result<Value, ClewError> {
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
    // Context projection is part of immutable Work identity. A saved exhaustive
    // section read ledger must not be reused as the compact packet's ledger.
    request.validate_documentation_language()?;
    normalize_section_profile(&subject, &mut request);
    validate_context_profile(&subject, &request)?;
    let selected = match kind {
        "service" if repo.services()?.contains_key(id) => BTreeSet::from([id.to_owned()]),
        "scenario"
            if repo.scenarios()?.contains_key(id)
                && (request.entrypoint.is_none()
                    || (request.entrypoint.as_deref() == Some(super::processes::OVERVIEW)
                        && repo.scenarios()?[id].process.is_some())
                    || (request.entrypoint.as_deref() == Some(super::dataflow::ROOT)
                        && repo.scenarios()?[id].view.is_some())) =>
        {
            BTreeSet::new()
        }
        _ => {
            return Err(invalid(
                "unknown work subject or unsupported scenario entrypoint",
            ));
        }
    };
    let (checked, evidence_snapshot) = Check::retained(repo, snapshot, &selected)?;
    let baseline = bindings::baseline(repo)?;
    if request.documentation_language.is_none() {
        request.documentation_language = baseline
            .as_ref()
            .and_then(|(_, binding)| binding.documentation_language.clone());
    }
    request.normalize_documentation_language()?;
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

    let component_scope = if kind == "scenario" {
        checked
            .scenarios
            .get(id)
            .map(|s| bindings::expand_dependencies(&s.dependency_ids, &checked))
            .transpose()?
            .unwrap_or_default()
    } else {
        BTreeSet::new()
    };
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
            checked
                .dependencies
                .iter()
                .filter(|(key, d)| {
                    !(d.kind.starts_with("PROCESS_") || d.kind.starts_with("VIEW_"))
                        || (kind == "scenario" && component_scope.contains(*key))
                })
                .map(|(id, _)| id.clone())
                .collect(),
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
    if kind == "scenario" && repo.scenarios()?[id].process.is_some() {
        for (index, root) in super::processes::expected(&checked, id)
            .into_iter()
            .filter(|root| root == id || root == super::processes::OVERVIEW)
            .enumerate()
        {
            handles.insert(
                format!("process{}", index + 1),
                Handle {
                    kind: "PROCESS_ROOT".into(),
                    id: root,
                },
            );
        }
    }
    let mut influence: BTreeMap<String, String> = checked
        .dependencies
        .iter()
        .filter(|(id, o)| {
            !(o.kind.starts_with("PROCESS_") || o.kind.starts_with("VIEW_"))
                || (kind == "scenario" && component_scope.contains(*id))
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
        snapshot: Some(evidence_snapshot.clone()),
        retained,
        external_inputs,
        handles,
        influence,
        obligations,
        review_reasons,
    };
    let mut stored = StoredWork::from_runtime(&work, evidence_snapshot);
    stored.id = digest(&stored)?[7..].into();
    // Validate selection before committing an unusable work object.
    rows(&work, &Selection::default())?;
    let encoded = bytes(&stored)?;
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
        let path = format!("{}/work.json", directory(&stored.id)?);
        if repo.path(&path)?.exists() {
            load(repo, &stored.id)?;
        } else {
            repo.atomic(&path, &encoded)?;
        }
    }
    work.id = stored.id;
    read_loaded(repo, &work, Selection::default())
}

// Only explicitly admitted root-relative files and the protected notes tree are read.
// The tree membership itself is captured, including absent notes and failed reads.
pub fn capture_inputs(
    repo: &Repository,
    request: &Request,
) -> Result<BTreeMap<String, Value>, ClewError> {
    capture_inputs_with_membership(repo, request).map(|(_, inputs)| inputs)
}

pub fn capture_inputs_with_membership(
    repo: &Repository,
    request: &Request,
) -> Result<(BTreeSet<String>, BTreeMap<String, Value>), ClewError> {
    if request.external_inputs.len() > 64 {
        return Err(invalid("select at most 64 external inputs"));
    }
    let mut note_paths = BTreeSet::new();
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
            note_paths.insert(relative);
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
            note_paths.insert(relative);
        }
    }
    let mut paths = note_paths.clone();
    paths.extend(request.external_inputs.iter().cloned());
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
    Ok((note_paths, out))
}

fn reference_roles(work: &Work, reference: &str) -> Vec<&'static str> {
    let Some(handle) = work.handles.get(reference) else {
        return Vec::new();
    };
    let mut roles = Vec::new();
    if super::proposals::evidence_reference_allowed(handle) {
        roles.push("evidence");
    }
    if super::proposals::operation_reference_allowed(work, handle) {
        roles.push("operation");
    }
    if super::proposals::gap_reference_allowed(handle) {
        roles.push("gap");
    }
    roles
}

fn section_content_preview(mut record: Value, max_bytes: usize) -> Result<Value, ClewError> {
    let budget = (max_bytes / 3).max(2048);
    if record["content"].is_null() || bytes(&record)?.len() <= budget {
        return Ok(record);
    }
    let content = record["content"].take();
    let text_preview = |value: &Value, chars| {
        value
            .as_str()
            .unwrap_or("")
            .chars()
            .take(chars)
            .collect::<String>()
    };
    let visual_count = content["visuals"].as_array().map_or(0, Vec::len);
    record["content"] = json!({
        "id":content["id"],"documentationLanguage":content["documentationLanguage"],"title":text_preview(&content["title"],128),
        "summary":{"text":text_preview(&content["summary"]["text"],512)},
        "visuals":[],"visualCount":visual_count,
        "eventCount":content["events"].as_array().map_or(0,Vec::len),
        "contractCount":content["interfaceContracts"].as_array().map_or(0,Vec::len)
    });
    record["contentProjection"] = json!({
        "kind":"RETAINED_ORIENTATION_ONLY","fullContentDeferred":true,
        "fullContentDigest":digest(&content)?,"fullContentBytes":bytes(&content)?.len(),
        "summaryMayBeTruncated":true,"deferredVisualDetails":visual_count,
        "nextRead":"Use docs section show for the full retained operation; then explicitly expand its source/dependency references before citing claims. This preview is not a replacement operation and does not authorize dropping retained artifacts."
    });
    if let Some(visuals) = content["visuals"].as_array() {
        for visual in visuals {
            let preview = json!({"id":visual["id"],"kind":visual["kind"],
                "title":text_preview(&visual["title"],80),
                "nodeCount":visual["nodes"].as_array().map_or(0,Vec::len),
                "edgeCount":visual["edges"].as_array().map_or(0,Vec::len),
                "ruleCount":visual["rules"].as_array().map_or(0,Vec::len)});
            record["content"]["visuals"]
                .as_array_mut()
                .unwrap()
                .push(preview);
            if bytes(&record)?.len() > budget {
                record["content"]["visuals"].as_array_mut().unwrap().pop();
                break;
            }
        }
    }
    record["contentProjection"]["omittedVisualIdentities"] =
        json!(visual_count.saturating_sub(record["content"]["visuals"].as_array().unwrap().len()));
    Ok(record)
}

// Initial context is an orientation packet, not an exhaustive evidence dump.
// Full evidence remains immutable and available through recorded expansions;
// preview limits never narrow the Work's conservative influence set.
fn compact_section_rows(
    work: &Work,
    service: &str,
    section: &str,
) -> Result<Vec<Value>, ClewError> {
    const PREVIEW_ITEMS: usize = 8;
    const PREVIEW_BYTES: usize = 8192;
    let mut rows: Vec<_> = super::sections::records(service, work.retained.as_ref())
        .into_iter()
        .filter(|record| record["id"] == section)
        .map(|record| Ok(json!({"kind":"SECTION","id":section,"record":section_content_preview(record,work.request.max_bytes)?})))
        .collect::<Result<_, ClewError>>()?;
    let mut inventory = super::sections::inventory(service, &work.checked);
    if let Some(public) = inventory["publicBoundaries"].as_array_mut() {
        let count = public.len();
        public.truncate(PREVIEW_ITEMS);
        inventory["publicBoundaryCount"] = json!(count);
        inventory["omittedPublicBoundaries"] = json!(count.saturating_sub(PREVIEW_ITEMS));
    }
    rows.push(
        json!({"kind":"BOUNDARY_INVENTORY","id":format!("inventory:{service}"),"record":inventory}),
    );
    let mut seeds = Vec::new();
    if let Some(operation) = work.retained.as_ref().and_then(|n| {
        n.operations
            .iter()
            .find(|operation| operation.id == section)
    }) {
        seeds.extend(
            operation
                .summary
                .dependency_ids
                .iter()
                .map(|id| (id, "retained-section-summary")),
        );
        for visual in &operation.visuals {
            for fragment in super::visuals::fragments(visual) {
                seeds.extend(
                    fragment
                        .dependency_ids
                        .iter()
                        .map(|id| (id, "retained-section-visual")),
                );
            }
        }
    }
    if let Some(evidence) = work.checked.services.get(service) {
        for entry in evidence.entrypoints.iter().take(PREVIEW_ITEMS) {
            seeds.extend(
                entry
                    .dependency_ids
                    .iter()
                    .filter(|id| {
                        work.checked.dependencies.get(*id).is_some_and(|d| {
                            matches!(
                                d.kind.as_str(),
                                "SYMBOL"
                                    | "SEMANTIC_SYMBOL"
                                    | "ENTRYPOINT"
                                    | "HTTP"
                                    | "ROUTE"
                                    | "SPRING_ROUTE"
                            )
                        })
                    })
                    .map(|id| (id, "discovered-entrypoint-declaration")),
            );
        }
    }
    let in_scope = |d: &&Observation| {
        (d.service == service || d.kind == "DOMAIN_ENTITY") && work.influence.contains_key(&d.id)
    };
    let mut counts = BTreeMap::<&str, usize>::new();
    for dependency in work.checked.dependencies.values().filter(in_scope) {
        *counts.entry(dependency.kind.as_str()).or_default() += 1;
    }
    let total: usize = counts.values().sum();
    let mut supplied = BTreeSet::new();
    let mut preview_bytes = 0usize;
    for (id, reason) in seeds {
        if supplied.len() == PREVIEW_ITEMS || supplied.contains(id) {
            continue;
        }
        let Some(dependency) = work.checked.dependencies.get(id).filter(in_scope) else {
            continue;
        };
        let record =
            json!({"kind":"DEPENDENCY","id":id,"record":dependency,"selectionReason":reason});
        let size = bytes(&record)?.len();
        if size > PREVIEW_BYTES.saturating_sub(preview_bytes) {
            continue;
        }
        preview_bytes += size;
        supplied.insert(id.clone());
        rows.push(record);
    }
    rows.push(json!({"kind":"EVIDENCE_DISCOVERY","id":format!("evidence-index:{service}"),"record":{
        "section":section,"availableDependencyCount":total,"countsByKind":counts,
        "suppliedDependencyCount":supplied.len(),"deferredDependencyCount":total.saturating_sub(supplied.len()),
        "previewLimits":{"maxItems":PREVIEW_ITEMS,"maxBytes":PREVIEW_BYTES},
        "selection":"Retained selected-section dependencies, then discovered entrypoint declarations. No arbitrary symbol sampling.",
        "nextRead":"Use sourceReferences/dependencyReferences with work expand, or a query with kind and symbolContains. Deferred facts are not absent or unsupported; read them before citing them.",
        "influenceCoverage":"Full captured Work influence is unchanged by this context preview."
    }}));
    Ok(rows)
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
    if work.request.context_profile.as_deref() == Some("declarations-v1")
        && selection.references.is_empty()
        && selection.symbols.is_empty()
        && selection.query.is_none()
    {
        return profile_rows(work);
    }
    let (kind, id) = work
        .subject
        .split_once(':')
        .ok_or_else(|| invalid("invalid stored work subject"))?;
    let process_root = selection.references.first().and_then(|reference| {
        work.handles
            .get(reference)
            .filter(|handle| handle.kind == "PROCESS_ROOT")
    });
    if let Some(handle) = process_root {
        if selection.references.len() != 1
            || !selection.symbols.is_empty()
            || selection.query.is_some()
        {
            return Err(invalid(
                "select a process section separately from evidence expansions",
            ));
        }
        return annotate_rows(work, vec![process_root_row(work, &handle.id)]);
    }
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
        let section = selection
            .references
            .first()
            .and_then(|reference| work.handles.get(reference))
            .map(|handle| handle.id.as_str())
            .or(work.request.entrypoint.as_deref())
            .ok_or_else(|| invalid("section selection requires an explicit section"))?;
        compact_section_rows(work, id, section)?
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
            snapshot: None,
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
        if kind == "scenario" {
            let roots: Vec<_> = work
                .handles
                .values()
                .filter(|handle| handle.kind == "PROCESS_ROOT")
                .map(|handle| process_root_row(work, &handle.id))
                .collect();
            items.splice(0..0, roots);
        }
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
    let projected = annotate_rows(work, items)?;
    if work.request.context_profile.as_deref() == Some(super::process_context::PROFILE)
        && selection.references.is_empty()
        && selection.symbols.is_empty()
        && selection.query.is_none()
    {
        super::process_context::project(projected)
    } else {
        Ok(projected)
    }
}

fn process_root_row(work: &Work, id: &str) -> Value {
    let authored = work.retained.as_ref().is_some_and(|narrative| {
        narrative
            .operations
            .iter()
            .any(|operation| operation.id == id)
    });
    json!({"kind":"PROCESS_ROOT","id":id,"record":{
        "subject":work.subject,"status":if authored {"AUTHORED"} else {"AWAITING_AUTHORING"},
        "purpose":if id == super::processes::OVERVIEW {"Cross-service process overview"} else {"Selected process behavior"},
        "instruction":"Use this work reference as a proposal operation or gap target; cite separately read source and dependency references for claims."
    }})
}

fn annotate_rows(work: &Work, mut items: Vec<Value>) -> Result<Vec<Value>, ClewError> {
    // Dynamic view/process facts outside this work subject are not supplied
    // or admitted as implicit influence of a service-only explanation.
    items.retain(|item| {
        item["kind"] != "DEPENDENCY"
            || item["id"]
                .as_str()
                .is_some_and(|id| work.influence.contains_key(id))
    });
    let reverse: BTreeMap<_, _> = work
        .handles
        .iter()
        .map(|(key, h)| ((h.kind.as_str(), h.id.as_str()), key.as_str()))
        .collect();
    for item in &mut items {
        if item["kind"] == "SECTION" {
            let accepted = item["record"]["content"]["documentationLanguage"].as_str();
            let status = if item["record"]["content"].is_null() {
                "NOT_AUTHORED"
            } else if accepted == Some(work.request.documentation_language()) {
                "MATCHES_REQUEST"
            } else {
                "REQUIRES_TRANSLATION"
            };
            item["documentationLanguageStatus"] = json!(status);
            item["requestedDocumentationLanguage"] = json!(work.request.documentation_language());
        }
        if let Some(reference) = reverse.get(&(
            item["kind"].as_str().unwrap_or(""),
            item["id"].as_str().unwrap_or(""),
        )) {
            item["reference"] = json!(reference);
        }
        item["referenceRoles"] = json!(
            item["reference"]
                .as_str()
                .map(|reference| reference_roles(work, reference))
                .unwrap_or_default()
        );
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

fn profile_rows(work: &Work) -> Result<Vec<Value>, ClewError> {
    let (kind, service) = work
        .subject
        .split_once(':')
        .ok_or_else(|| invalid("invalid stored work subject"))?;
    if kind != "service" {
        return Err(invalid("declarations-v1 requires a service work subject"));
    }
    let mut items = Vec::new();
    let mut selected_membership = Vec::new();
    let mut deferred_membership = Vec::new();
    let mut add = |kind: &str, id: String, record: Value| {
        let item_id = id.clone();
        selected_membership.push(json!([kind, id]));
        items.push(json!({"kind":kind,"id":item_id,"record":record}));
    };

    if let Some(section) = super::sections::records(service, work.retained.as_ref())
        .into_iter()
        .find(|record| record["id"] == "section-entities")
    {
        add(
            "SECTION",
            section["id"].as_str().unwrap_or_default().to_owned(),
            section,
        );
    }
    let mut source_ids = BTreeSet::new();
    for observation in work.checked.dependencies.values() {
        let in_scope = observation.service == service || observation.kind == "DOMAIN_ENTITY";
        if !in_scope {
            continue;
        }
        if matches!(observation.kind.as_str(), "SYNTAX_DETAIL" | "FLOW") {
            deferred_membership.push(json!(["DEPENDENCY", observation.id]));
            continue;
        }
        if !work.influence.contains_key(&observation.id) {
            continue;
        }
        for source_id in &observation.source_ids {
            source_ids.insert(source_id.clone());
        }
        add(
            "DEPENDENCY",
            observation.id.clone(),
            serde_json::to_value(observation).map_err(io_error)?,
        );
    }
    let sources = work.checked.sources();
    for source_id in source_ids {
        let source = sources.get(&source_id).ok_or_else(|| {
            invalid(format!(
                "CONTEXT_PROFILE_MISSING_SOURCE: selected dependency references missing source {source_id}"
            ))
        })?;
        add(
            "SOURCE",
            source_id,
            serde_json::to_value(source).map_err(io_error)?,
        );
    }
    for (index, record) in work.review_reasons.iter().enumerate() {
        add("REVIEW_REASON", format!("review-{index}"), record.clone());
    }
    for (path, record) in &work.external_inputs {
        add("EXTERNAL_INPUT", path.clone(), record.clone());
    }
    for (index, obligation) in work.obligations.iter().enumerate() {
        add(
            "OBLIGATION",
            format!("obligation-{}", index + 1),
            obligation.clone(),
        );
    }
    let deferred_sections: Vec<_> = super::sections::records(service, work.retained.as_ref())
        .into_iter()
        .filter(|section| section["id"] != "section-entities")
        .map(|section| {
            json!({
                "id": section["id"],
                "required": section["required"],
                "status": section["status"],
            })
        })
        .collect();
    for section in &deferred_sections {
        deferred_membership.push(json!(["SECTION", section["id"]]));
    }
    deferred_membership.push(json!([
        "BOUNDARY_INVENTORY",
        format!("inventory:{service}")
    ]));

    let inventory = super::sections::inventory(service, &work.checked);
    let known_section_references: Vec<_> = work
        .handles
        .iter()
        .filter(|(_, handle)| handle.kind == "SECTION")
        .take(5)
        .map(|(reference, _)| reference.clone())
        .collect();
    let inventory_digest = digest(&inventory)?;
    let summary = json!({
        "profile": "declarations-v1",
        "focus": "section-entities",
        "authority": "IMMUTABLE_WORK_CAPTURE_NOT_REVERIFIED",
        "selectedCount": selected_membership.len(),
        "deferredCount": deferred_membership.len(),
        "selectedMembershipDigest": digest(&selected_membership)?,
        "deferredMembershipDigest": digest(&deferred_membership)?,
        "coverage": work.checked.services.get(service).map(|e| e.coverage.clone()).unwrap_or_default(),
        "gaps": inventory["gaps"],
        "sourceBoundaries": inventory["sourceBoundaries"],
        "deferredSections": deferred_sections,
        "inventoryDigest": inventory_digest.clone(),
        "inventory": {
            "publicBoundaries": inventory["publicBoundaries"].as_array().map_or(0, |v| v.len()),
            "internalCallables": inventory["internalCallableCount"].as_u64().unwrap_or_else(|| inventory["internalCallables"].as_array().map_or(0, |v| v.len()) as u64),
            "digest": inventory_digest,
        },
        "expansion": {
            "kind": "*",
            "query": {"kind":"*","symbolContains":""},
            "sectionReferences": known_section_references,
        },
        "deferredByProfile": true,
    });
    items.push(json!({
        "kind":"CONTEXT_PROFILE",
        "id":"context-profile:declarations-v1",
        "record":summary,
    }));
    annotate_rows(work, items)
}

pub fn read(repo: &Repository, id: &str, selection: Selection) -> Result<Value, ClewError> {
    let work = load(repo, id)?;
    read_loaded(repo, &work, selection)
}

pub(super) fn read_loaded(
    repo: &Repository,
    work: &Work,
    selection: Selection,
) -> Result<Value, ClewError> {
    let id = work.id.as_str();
    let items = super::progress::run("BUILD_CONTEXT_ROWS", || rows(work, &selection))?;
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
    output["documentationLanguage"] = json!(work.request.documentation_language());
    if work.subject.starts_with("scenario:") {
        output["subjectReference"] = json!({
            "reference": work.subject,
            "referenceRoles": ["operation", "gap"]
        });
    }
    if let Some(snapshot) = &work.snapshot {
        output["snapshot"] = json!(snapshot);
    }
    if let Some(profile) = &work.request.context_profile {
        output["contextProfile"] = json!(profile);
    }
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
    let _lock = super::progress::run("WAIT_READ_RECEIPT_LOCK", || repo.lock())?;
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
    super::progress::run("SAVE_READ_RECEIPT", || {
        repo.atomic(&format!("{}/reads.json", directory(id)?), &encoded)
    })?;
    Ok(output)
}

/// All initially required facts, old content and human inputs must be supplied.
/// Omitted records are an evidence-budget problem, never a model-reasoning problem.
pub fn initial_context_complete(state: &ReadState) -> bool {
    let mut cursor: Option<String> = None;
    let mut membership: Option<String> = None;
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
        if membership
            .as_deref()
            .is_some_and(|digest| digest != receipt.membership_digest)
        {
            return false;
        }
        membership = Some(receipt.membership_digest.clone());
        if receipt.next_cursor.is_none() {
            return true;
        }
        cursor = receipt.next_cursor.clone();
    }
    false
}

#[cfg(test)]
mod section_context_tests {
    use super::*;

    fn fixture(count: usize, retained: bool) -> Work {
        let mut dependencies = BTreeMap::new();
        let mut handles = BTreeMap::from([
            (
                "section2".into(),
                Handle {
                    kind: "SECTION".into(),
                    id: "section-responsibilities".into(),
                },
            ),
            (
                "s1".into(),
                Handle {
                    kind: "SOURCE".into(),
                    id: "retained-source".into(),
                },
            ),
        ]);
        let mut influence = BTreeMap::new();
        for i in 0..count {
            let id = format!("orders:symbol:{i:05}");
            let normalized = json!({"name":format!("Handler{i}"),"kind":"function"});
            let fact_digest = digest(&normalized).unwrap();
            dependencies.insert(
                id.clone(),
                Observation {
                    id: id.clone(),
                    kind: "SYMBOL".into(),
                    service: "orders".into(),
                    symbol: format!("Handler{i}"),
                    digest: fact_digest.clone(),
                    normalized,
                    source_ids: vec!["retained-source".into()],
                },
            );
            handles.insert(
                format!("d{i}"),
                Handle {
                    kind: "DEPENDENCY".into(),
                    id: id.clone(),
                },
            );
            influence.insert(id, fact_digest);
        }
        let checked: Check = serde_json::from_value(json!({
            "schema":"codeclew-documentation-check/1.0","inputDigest":"input","contextDigest":"context",
            "services":{},"unresolved":{},"interactions":{},"scenarios":{},"dependencies":dependencies
        })).unwrap();
        let narrative = retained.then(|| serde_json::from_value(json!({
            "schema":"codeclew-documentation-narrative/1.3","subject":"service:orders","contextDigest":"context",
            "operations":[{"id":"section-responsibilities","title":"Responsibilities","participants":[],"events":[],
                "summary":{"id":"summary","text":"Explain the selected handler.",
                    "dependencyIds":[format!("orders:symbol:{:05}",count-1)],"sourceIds":["retained-source"]}}]
        })).unwrap());
        Work {
            schema: WORK_SCHEMA.into(),
            id: "work".into(),
            subject: "service:orders".into(),
            request: Request {
                schema: "codeclew-documentation-work-request/1.0".into(),
                audience: "Maintainers".into(),
                documentation_language: None,
                entrypoint: Some("section-responsibilities".into()),
                context_profile: None,
                max_items: 100,
                max_bytes: 49152,
                external_inputs: vec![],
            },
            checked,
            snapshot: None,
            retained: narrative,
            external_inputs: BTreeMap::new(),
            handles,
            influence,
            obligations: vec![],
            review_reasons: vec![],
        }
    }

    #[test]
    fn documentation_language_changes_work_identity_without_changing_evidence() {
        let mut work = fixture(1, true);
        let old = StoredWork::from_runtime(&work, "capture".into());
        assert!(
            serde_json::to_value(&old).unwrap()["request"]
                .get("documentationLanguage")
                .is_none()
        );
        work.request.normalize_documentation_language().unwrap();
        assert_eq!(work.request.documentation_language(), "en");
        let english = StoredWork::from_runtime(&work, "capture".into());
        work.request.documentation_language = Some("ru".into());
        let russian = StoredWork::from_runtime(&work, "capture".into());
        assert_ne!(digest(&old).unwrap(), digest(&english).unwrap());
        assert_ne!(digest(&english).unwrap(), digest(&russian).unwrap());
        assert_eq!(english.evidence_snapshot, russian.evidence_snapshot);
        assert_eq!(english.influence, russian.influence);
        assert_eq!(english.retained, russian.retained);
        let rows = rows(&work, &Selection::default()).unwrap();
        let section = rows.iter().find(|row| row["kind"] == "SECTION").unwrap();
        assert_eq!(
            section["documentationLanguageStatus"],
            "REQUIRES_TRANSLATION"
        );
        work.retained.as_mut().unwrap().operations[0].documentation_language = Some("en".into());
        assert_eq!(rows_for_section_language(&work), "REQUIRES_TRANSLATION");
        work.retained.as_mut().unwrap().operations[0].documentation_language = Some("ru".into());
        assert_eq!(rows_for_section_language(&work), "MATCHES_REQUEST");
        work.request.documentation_language = Some("de".into());
        assert!(work.request.normalize_documentation_language().is_err());
    }

    fn rows_for_section_language(work: &Work) -> Value {
        rows(work, &Selection::default())
            .unwrap()
            .into_iter()
            .find(|row| row["kind"] == "SECTION")
            .unwrap()["documentationLanguageStatus"]
            .clone()
    }

    #[test]
    fn documentation_language_flag_cannot_override_conflicting_input() {
        let request = fixture(1, false).request;
        let russian = request.with_language_flag(Some("ru".into())).unwrap();
        assert_eq!(russian.documentation_language(), "ru");
        assert!(
            russian
                .clone()
                .with_language_flag(Some("en".into()))
                .is_err()
        );
        assert!(russian.with_language_flag(Some("ru".into())).is_ok());
    }

    #[test]
    fn section_projection_version_separates_old_and_new_immutable_work_ledgers() {
        let mut work = fixture(1, false);
        let old = StoredWork::from_runtime(&work, "capture".into());
        let old_bytes = bytes(&old).unwrap();
        let old_id = digest(&old).unwrap();
        normalize_section_profile(&work.subject, &mut work.request);
        assert_eq!(
            work.request.context_profile.as_deref(),
            Some(SECTION_ORIENTATION_PROFILE)
        );
        validate_context_profile(&work.subject, &work.request).unwrap();
        let current = StoredWork::from_runtime(&work, "capture".into());
        assert_ne!(digest(&current).unwrap(), old_id);
        assert_eq!(bytes(&old).unwrap(), old_bytes);
        let current_id = digest(&current).unwrap();
        normalize_section_profile(&work.subject, &mut work.request);
        assert_eq!(
            digest(&StoredWork::from_runtime(&work, "capture".into())).unwrap(),
            current_id
        );
        work.request.entrypoint = None;
        assert!(validate_context_profile(&work.subject, &work.request).is_err());
        assert!(validate_context_profile("scenario:process", &current.request).is_err());
    }

    #[test]
    fn large_section_initial_context_uses_exact_retained_seeds_not_all_dependencies() {
        let work = fixture(20_000, true);
        let original_influence = work.influence.clone();
        let rows = rows(&work, &Selection::default()).unwrap();
        assert_eq!(rows.iter().filter(|r| r["kind"] == "SECTION").count(), 1);
        let dependencies: Vec<_> = rows.iter().filter(|r| r["kind"] == "DEPENDENCY").collect();
        assert_eq!(dependencies.len(), 1);
        assert_eq!(dependencies[0]["reference"], "d19999");
        assert_eq!(dependencies[0]["sourceReferences"], json!(["s1"]));
        let discovery = rows
            .iter()
            .find(|r| r["kind"] == "EVIDENCE_DISCOVERY")
            .unwrap();
        assert_eq!(discovery["record"]["availableDependencyCount"], 20_000);
        assert_eq!(discovery["record"]["deferredDependencyCount"], 19_999);
        assert!(bytes(&rows).unwrap().len() < 8192);
        assert_eq!(work.influence, original_influence);
    }

    #[test]
    fn large_retained_visuals_reprepare_as_orientation_without_mutating_content() {
        let mut work = fixture(10, true);
        let fragment = |id: String, text: String| {
            json!({"id":id,"text":text,
            "dependencyIds":["orders:symbol:00009"],"sourceIds":["retained-source"]})
        };
        let visuals = (0..2).map(|g| serde_json::from_value(json!({
            "schema":super::super::visuals::SCHEMA,"generator":super::super::visuals::GENERATOR,
            "id":format!("flow{g}"),"kind":"execution-flow","title":format!("Retained flow {g}"),
            "purpose":fragment(format!("purpose{g}"),"Explain retained execution".into()),
            "scope":fragment(format!("scope{g}"),"One retained method".into()),"limitations":["Static interpretation only."],
            "nodes":(0..64).map(|n|json!({"id":format!("node{n}"),"meaning":fragment(format!("node{g}-{n}"),"x".repeat(1024))})).collect::<Vec<_>>()
        })).unwrap()).collect::<Vec<super::super::visuals::Visual>>();
        super::super::visuals::validate_structure(&visuals).unwrap();
        work.retained.as_mut().unwrap().operations[0].visuals = visuals;
        let original = bytes(&work.retained).unwrap();
        assert!(original.len() > work.request.max_bytes);
        let initial = rows(&work, &Selection::default()).unwrap();
        assert!(bytes(&initial).unwrap().len() < work.request.max_bytes);
        let section = initial.iter().find(|r| r["kind"] == "SECTION").unwrap();
        assert_eq!(
            section["record"]["contentProjection"]["fullContentDeferred"],
            true
        );
        assert_eq!(section["record"]["content"]["visualCount"], 2);
        assert_eq!(section["record"]["content"]["visuals"][0]["id"], "flow0");
        assert!(
            section["record"]["content"]["visuals"][0]
                .get("nodes")
                .is_none()
        );
        assert_eq!(bytes(&work.retained).unwrap(), original);
        let explicit = rows(
            &work,
            &Selection {
                references: vec!["section2".into()],
                ..Selection::default()
            },
        )
        .unwrap();
        assert!(bytes(&explicit).unwrap().len() < work.request.max_bytes);
    }

    #[test]
    fn unseeded_sections_explain_discovery_without_random_fact_sampling() {
        let work = fixture(20_000, false);
        let initial = rows(&work, &Selection::default()).unwrap();
        assert!(!initial.iter().any(|r| r["kind"] == "DEPENDENCY"));
        let expanded = rows(
            &work,
            &Selection {
                query: Some(Query {
                    kind: "SYMBOL".into(),
                    symbol_contains: "Handler19999".into(),
                }),
                ..Selection::default()
            },
        )
        .unwrap();
        assert_eq!(expanded.len(), 1);
        assert_eq!(expanded[0]["reference"], "d19999");
        let selected = rows(
            &work,
            &Selection {
                references: vec!["section2".into()],
                ..Selection::default()
            },
        )
        .unwrap();
        assert!(selected.len() < 10);
        assert_eq!(
            selected.iter().find(|r| r["kind"] == "SECTION").unwrap()["id"],
            "section-responsibilities"
        );
    }
}
