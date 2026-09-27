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

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Query {
    pub kind: String,
    #[serde(default)]
    pub symbol_contains: String,
    #[serde(default, skip_serializing_if = "QueryProjection::is_raw")]
    pub projection: QueryProjection,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum QueryProjection {
    #[default]
    Raw,
    Navigation,
}

impl QueryProjection {
    fn is_raw(&self) -> bool {
        *self == Self::Raw
    }
}

pub(super) fn validate_selection(selection: &Selection) -> Result<(), ClewError> {
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
    if selection.query.as_ref().is_some_and(|query| {
        query.projection == QueryProjection::Navigation && query.kind != "SYMBOL"
    }) {
        return Err(invalid("NAVIGATION query projection requires kind SYMBOL"));
    }
    Ok(())
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
const SECTION_ENTITIES_ORIENTATION_PROFILE: &str = "section-entities-orientation-v1";
const HTTP_API_CONTRACT_PROFILE: &str = "http-api-contract-v1";
const JAVA_COMPILER_FACT_SCHEMA: &str = "codeclew-java-compiler-fact/1.0";
const MAX_DIRECT_API_TYPES: usize = 8;
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_selection: Option<Selection>,
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
    validate_http_api_contract_profile(&stored.subject, &stored.request, &checked)?;
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
        request.context_profile = Some(
            if request.entrypoint.as_deref() == Some("section-entities") {
                SECTION_ENTITIES_ORIENTATION_PROFILE
            } else {
                SECTION_ORIENTATION_PROFILE
            }
            .into(),
        );
    }
}

fn is_java_http_entrypoint(checked: &Check, subject: &str, request: &Request) -> bool {
    let (Some(service), Some(entrypoint_id)) = (
        subject.strip_prefix("service:"),
        request.entrypoint.as_deref(),
    ) else {
        return false;
    };
    let Some(evidence) = checked.services.get(service) else {
        return false;
    };
    if evidence.extractor != EXTRACTOR {
        return false;
    }
    let mut entries = evidence
        .entrypoints
        .iter()
        .filter(|entry| entry.id == entrypoint_id && entry.kind == "HTTP_ENDPOINT");
    let Some(entry) = entries.next() else {
        return false;
    };
    if entries.next().is_some() {
        return false;
    }
    entry.dependency_ids.iter().any(|id| {
        checked.dependencies.get(id).is_some_and(|declaration| {
            declaration.service == service
                && declaration.kind == "SYMBOL"
                && declaration.symbol == entry.symbol
                && declaration.normalized["symbolIdentity"] == entry.symbol
                && declaration.normalized["schema"] == JAVA_COMPILER_FACT_SCHEMA
                && declaration.normalized["declarationKind"] == "METHOD"
        })
    })
}

fn normalize_http_api_contract_profile(subject: &str, request: &mut Request, checked: &Check) {
    if request.context_profile.is_none() && is_java_http_entrypoint(checked, subject, request) {
        request.context_profile = Some(HTTP_API_CONTRACT_PROFILE.into());
    }
}

fn validate_http_api_contract_profile(
    subject: &str,
    request: &Request,
    checked: &Check,
) -> Result<(), ClewError> {
    if request.context_profile.as_deref() == Some(HTTP_API_CONTRACT_PROFILE)
        && !is_java_http_entrypoint(checked, subject, request)
    {
        return Err(invalid(
            "CONTEXT_PROFILE_INCOMPATIBLE: http-api-contract-v1 requires one captured Java HTTP endpoint",
        ));
    }
    Ok(())
}

fn descriptor_object_types(descriptor: &str) -> Result<BTreeSet<String>, ()> {
    fn type_at(input: &[u8], cursor: &mut usize, allow_void: bool) -> Result<Option<String>, ()> {
        let mut dimensions = 0usize;
        while input.get(*cursor) == Some(&b'[') {
            dimensions += 1;
            if dimensions > 255 {
                return Err(());
            }
            *cursor += 1;
        }
        match input.get(*cursor).copied() {
            Some(b'V') if allow_void && dimensions == 0 => {
                *cursor += 1;
                Ok(None)
            }
            Some(b'B' | b'C' | b'D' | b'F' | b'I' | b'J' | b'S' | b'Z') => {
                *cursor += 1;
                Ok(None)
            }
            Some(b'L') => {
                *cursor += 1;
                let start = *cursor;
                let end = input[start..]
                    .iter()
                    .position(|byte| *byte == b';')
                    .map(|offset| start + offset)
                    .ok_or(())?;
                let binary_name = std::str::from_utf8(&input[start..end]).map_err(|_| ())?;
                if binary_name.is_empty()
                    || binary_name.starts_with('/')
                    || binary_name.ends_with('/')
                    || binary_name.split('/').any(str::is_empty)
                    || binary_name
                        .bytes()
                        .any(|byte| matches!(byte, b'.' | b';' | b'['))
                {
                    return Err(());
                }
                *cursor = end + 1;
                Ok(Some(format!("class:{}", binary_name.replace('/', "."))))
            }
            _ => Err(()),
        }
    }

    let input = descriptor.as_bytes();
    if input.first() != Some(&b'(') {
        return Err(());
    }
    let mut cursor = 1usize;
    let mut types = BTreeSet::new();
    let mut parameter_slots = 0usize;
    while input.get(cursor) != Some(&b')') {
        let start = cursor;
        let identity = type_at(input, &mut cursor, false)?;
        if cursor <= start {
            return Err(());
        }
        parameter_slots += if input[start..cursor].first() == Some(&b'[') {
            1
        } else if matches!(input[start], b'J' | b'D') {
            2
        } else {
            1
        };
        if parameter_slots > 255 {
            return Err(());
        }
        types.extend(identity);
    }
    cursor += 1;
    let return_identity = type_at(input, &mut cursor, true)?;
    if cursor != input.len() {
        return Err(());
    }
    types.extend(return_identity);
    Ok(types)
}

fn http_api_contract_preparation(work: &Work) -> Result<(Vec<Value>, Value), ClewError> {
    let (service, entrypoint_id) = work
        .subject
        .strip_prefix("service:")
        .and_then(|service| {
            work.request
                .entrypoint
                .as_deref()
                .map(|entrypoint| (service, entrypoint))
        })
        .unwrap_or_default();
    let mut selected_rows = Vec::new();
    let mut limitations = vec![json!({
        "code":"DIRECT_DECLARED_TYPES_ONLY",
        "detail":"Only direct declared parameter and return object types are selected. This does not establish inherited fields, validation activation, wire requiredness, serialization, generic payloads, or complete contracts."
    })];
    let mut direct_type_count = 0usize;
    let mut selected_declaration_count = 0usize;

    let entry = work.checked.services.get(service).and_then(|evidence| {
        let mut matches = evidence
            .entrypoints
            .iter()
            .filter(|entry| entry.id == entrypoint_id && entry.kind == "HTTP_ENDPOINT");
        let entry = matches.next()?;
        matches.next().is_none().then_some(entry)
    });
    let Some(entry) = entry else {
        limitations.push(json!({"code":"HTTP_ENDPOINT_UNAVAILABLE"}));
        return Ok((
            selected_rows,
            http_api_contract_obligation(
                service,
                entrypoint_id,
                direct_type_count,
                selected_declaration_count,
                limitations,
            ),
        ));
    };

    let declarations: Vec<_> = entry
        .dependency_ids
        .iter()
        .filter_map(|id| work.checked.dependencies.get(id))
        .filter(|declaration| {
            declaration.service == service
                && declaration.kind == "SYMBOL"
                && declaration.symbol == entry.symbol
                && declaration.normalized["symbolIdentity"] == entry.symbol
                && declaration.normalized["schema"] == JAVA_COMPILER_FACT_SCHEMA
                && declaration.normalized["declarationKind"] == "METHOD"
        })
        .collect();
    let declaration = match declarations.as_slice() {
        [declaration] if work.influence.contains_key(&declaration.id) => *declaration,
        [] => {
            limitations.push(json!({"code":"JAVA_ENDPOINT_DECLARATION_MISSING"}));
            return Ok((
                selected_rows,
                http_api_contract_obligation(
                    service,
                    entrypoint_id,
                    direct_type_count,
                    selected_declaration_count,
                    limitations,
                ),
            ));
        }
        [_] => {
            limitations.push(json!({"code":"JAVA_ENDPOINT_OUTSIDE_WORK_INFLUENCE"}));
            return Ok((
                selected_rows,
                http_api_contract_obligation(
                    service,
                    entrypoint_id,
                    direct_type_count,
                    selected_declaration_count,
                    limitations,
                ),
            ));
        }
        _ => {
            limitations.push(json!({"code":"JAVA_ENDPOINT_DECLARATION_AMBIGUOUS"}));
            return Ok((
                selected_rows,
                http_api_contract_obligation(
                    service,
                    entrypoint_id,
                    direct_type_count,
                    selected_declaration_count,
                    limitations,
                ),
            ));
        }
    };
    let Some(scope) = declaration.normalized["scope"]
        .as_str()
        .filter(|scope| !scope.is_empty())
    else {
        limitations.push(json!({"code":"JAVA_ENDPOINT_COMPILATION_SCOPE_UNAVAILABLE"}));
        return Ok((
            selected_rows,
            http_api_contract_obligation(
                service,
                entrypoint_id,
                direct_type_count,
                selected_declaration_count,
                limitations,
            ),
        ));
    };
    let Some(descriptor) = declaration.normalized["jvmDescriptor"].as_str() else {
        limitations.push(json!({"code":"JVM_METHOD_DESCRIPTOR_UNAVAILABLE"}));
        return Ok((
            selected_rows,
            http_api_contract_obligation(
                service,
                entrypoint_id,
                direct_type_count,
                selected_declaration_count,
                limitations,
            ),
        ));
    };
    let direct_types = match descriptor_object_types(descriptor) {
        Ok(types) => types,
        Err(()) => {
            limitations.push(json!({"code":"JVM_METHOD_DESCRIPTOR_UNSUPPORTED_OR_MALFORMED"}));
            return Ok((
                selected_rows,
                http_api_contract_obligation(
                    service,
                    entrypoint_id,
                    direct_type_count,
                    selected_declaration_count,
                    limitations,
                ),
            ));
        }
    };
    direct_type_count = direct_types.len();
    if direct_types.len() > MAX_DIRECT_API_TYPES {
        limitations.push(json!({
            "code":"DIRECT_TYPE_BOUND_DEFERRED",
            "maxDistinctTypes":MAX_DIRECT_API_TYPES,
            "observedDistinctTypes":direct_types.len(),
        }));
        return Ok((
            selected_rows,
            http_api_contract_obligation(
                service,
                entrypoint_id,
                direct_type_count,
                selected_declaration_count,
                limitations,
            ),
        ));
    }

    let sources = &work.checked.services[service].sources;
    let mut seen = BTreeSet::new();
    for identity in direct_types {
        let candidates: Vec<_> = work
            .checked
            .dependencies
            .values()
            .filter(|candidate| {
                candidate.service == service
                    && candidate.kind == "SYMBOL"
                    && candidate.normalized["schema"] == JAVA_COMPILER_FACT_SCHEMA
                    && matches!(
                        candidate.normalized["declarationKind"].as_str(),
                        Some("CLASS" | "INTERFACE" | "ENUM" | "RECORD" | "ANNOTATION")
                    )
                    && candidate.symbol == identity
                    && candidate.normalized["symbolIdentity"] == identity
                    && candidate.normalized["scope"].as_str() == Some(scope)
                    && work.influence.contains_key(&candidate.id)
            })
            .collect();
        let candidate = match candidates.as_slice() {
            [candidate] => *candidate,
            [] => {
                let unsupported = work.checked.dependencies.values().any(|candidate| {
                    candidate.service == service
                        && candidate.kind == "SYMBOL"
                        && (candidate.symbol == identity
                            || candidate.normalized["symbolIdentity"] == identity)
                        && candidate.normalized["scope"].as_str() == Some(scope)
                        && work.influence.contains_key(&candidate.id)
                });
                limitations.push(json!({
                    "code":if unsupported {"DIRECT_TYPE_METADATA_UNSUPPORTED"} else {"DIRECT_TYPE_DECLARATION_MISSING"},
                    "symbolIdentity":identity,
                }));
                continue;
            }
            _ => {
                limitations.push(json!({
                    "code":"DIRECT_TYPE_DECLARATION_AMBIGUOUS",
                    "symbolIdentity":identity,
                }));
                continue;
            }
        };
        if seen.insert(("DEPENDENCY", candidate.id.as_str())) {
            selected_rows.push(json!({"kind":"DEPENDENCY","id":candidate.id,"record":candidate}));
        }
        if candidate.source_ids.is_empty() {
            limitations.push(json!({
                "code":"DIRECT_TYPE_SOURCE_UNAVAILABLE",
                "symbolIdentity":identity,
            }));
        }
        for source_id in &candidate.source_ids {
            match sources
                .get(source_id)
                .filter(|source| source.service == service)
            {
                Some(source) if seen.insert(("SOURCE", source_id.as_str())) => {
                    selected_rows.push(json!({"kind":"SOURCE","id":source_id,"record":source}));
                }
                Some(_) => {}
                None => limitations.push(json!({
                    "code":"DIRECT_TYPE_SOURCE_RECORD_MISSING",
                    "symbolIdentity":identity,
                    "sourceId":source_id,
                })),
            }
        }
    }
    selected_declaration_count = selected_rows
        .iter()
        .filter(|row| row["kind"] == "DEPENDENCY")
        .count();
    Ok((
        selected_rows,
        http_api_contract_obligation(
            service,
            entrypoint_id,
            direct_type_count,
            selected_declaration_count,
            limitations,
        ),
    ))
}

fn http_api_contract_obligation(
    service: &str,
    entrypoint: &str,
    direct_type_count: usize,
    selected_declaration_count: usize,
    limitations: Vec<Value>,
) -> Value {
    json!({
        "kind":"HTTP_API_CONTRACT_PREPARATION",
        "service":service,
        "entrypoint":entrypoint,
        "directDescriptorTypeCount":direct_type_count,
        "selectedDeclarationCount":selected_declaration_count,
        "limitations":limitations,
    })
}

fn validate_context_profile(subject: &str, request: &Request) -> Result<(), ClewError> {
    match request.context_profile.as_deref() {
        None if subject.starts_with("service:")
            && request.entrypoint.as_deref() == Some("section-entities") =>
        {
            Err(invalid(
                "CONTEXT_PROFILE_INCOMPATIBLE: legacy section-entities Work must be prepared again from its retained snapshot; reindexing is not required",
            ))
        }
        None => Ok(()),
        Some(SECTION_ORIENTATION_PROFILE)
            if request.entrypoint.as_deref() == Some("section-entities") =>
        {
            Err(invalid(
                "CONTEXT_PROFILE_INCOMPATIBLE: section-entities projection changed; prepare a fresh Work from its retained snapshot, without reindexing",
            ))
        }
        Some(SECTION_ORIENTATION_PROFILE)
            if subject.starts_with("service:")
                && request.entrypoint.as_deref().is_some_and(|section| {
                    super::sections::contains(section) && section != "section-entities"
                }) =>
        {
            Ok(())
        }
        Some(SECTION_ORIENTATION_PROFILE) => Err(invalid(
            "CONTEXT_PROFILE_INCOMPATIBLE: section-orientation-v1 requires a non-entity service section",
        )),
        Some(SECTION_ENTITIES_ORIENTATION_PROFILE)
            if subject.starts_with("service:")
                && request.entrypoint.as_deref() == Some("section-entities") =>
        {
            Ok(())
        }
        Some(SECTION_ENTITIES_ORIENTATION_PROFILE) => Err(invalid(
            "CONTEXT_PROFILE_INCOMPATIBLE: section-entities-orientation-v1 requires a service section-entities request",
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
        Some(HTTP_API_CONTRACT_PROFILE)
            if subject.starts_with("service:") && request.entrypoint.is_some() =>
        {
            Ok(())
        }
        Some(HTTP_API_CONTRACT_PROFILE) => Err(invalid(
            "CONTEXT_PROFILE_INCOMPATIBLE: http-api-contract-v1 requires a service endpoint request",
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
    normalize_http_api_contract_profile(&subject, &mut request, &checked);
    validate_context_profile(&subject, &request)?;
    validate_http_api_contract_profile(&subject, &request, &checked)?;
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
    if work.request.context_profile.as_deref() == Some(HTTP_API_CONTRACT_PROFILE) {
        let (_, obligation) = http_api_contract_preparation(&work)?;
        work.obligations.push(obligation);
    }
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
    let entity_orientation = section == "section-entities"
        && work.request.context_profile.as_deref() == Some(SECTION_ENTITIES_ORIENTATION_PROFILE);
    let entity_related_to_service = |dependency: &Observation| {
        dependency.kind == "DOMAIN_ENTITY"
            && dependency.normalized["entity"]["relations"]
                .as_array()
                .is_some_and(|relations| {
                    relations
                        .iter()
                        .any(|relation| relation["service"] == service)
                })
    };
    let mut seeds = Vec::new();
    if entity_orientation {
        seeds.extend(
            work.checked
                .dependencies
                .values()
                .filter(|dependency| {
                    dependency.kind == "ENTITY_SCOPE" && dependency.service == service
                })
                .map(|dependency| (&dependency.id, "selected-service-entity-scope")),
        );
        seeds.extend(
            work.checked
                .dependencies
                .values()
                .filter(|dependency| entity_related_to_service(dependency))
                .map(|dependency| (&dependency.id, "service-related-domain-entity")),
        );
    }
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
        let selected = if entity_orientation {
            d.service == service || (d.kind == "DOMAIN_ENTITY" && entity_related_to_service(d))
        } else {
            d.service == service || d.kind == "DOMAIN_ENTITY"
        };
        selected && work.influence.contains_key(&d.id)
    };
    let mut counts = BTreeMap::<&str, usize>::new();
    for dependency in work.checked.dependencies.values().filter(in_scope) {
        *counts.entry(dependency.kind.as_str()).or_default() += 1;
    }
    let total: usize = counts.values().sum();
    let mut supplied = BTreeSet::new();
    let mut navigation_only = BTreeSet::new();
    let mut preview_items = 0usize;
    let mut preview_bytes = 0usize;
    let entity_reverse = entity_orientation.then(|| reverse_handles(work));
    for (id, reason) in seeds {
        if preview_items == PREVIEW_ITEMS || supplied.contains(id) || navigation_only.contains(id) {
            continue;
        }
        let Some(dependency) = work.checked.dependencies.get(id).filter(in_scope) else {
            continue;
        };
        let record =
            json!({"kind":"DEPENDENCY","id":id,"record":dependency,"selectionReason":reason});
        let size = if let Some(reverse) = &entity_reverse {
            bytes(&annotate_rows_with_reverse(work, vec![record.clone()], reverse)?[0])?.len()
        } else {
            bytes(&record)?.len()
        };
        if size > PREVIEW_BYTES.saturating_sub(preview_bytes) {
            if entity_orientation
                && matches!(dependency.kind.as_str(), "ENTITY_SCOPE" | "DOMAIN_ENTITY")
            {
                let navigation = json!({
                    "kind":"ENTITY_NAVIGATION",
                    "id":format!("entity-navigation:{id}"),
                    "selectionReason":reason,
                    "record":{
                        "kind":dependency.kind,
                        "dependencyKind":dependency.kind,
                        "symbol":dependency.symbol,
                        "limitation":"The full declaration is deferred by the bounded initial preview; expand dependencyReferences before relying on its contents.",
                        "normalized":{"dependencyIds":[id]}
                    }
                });
                let navigation_size = if let Some(reverse) = &entity_reverse {
                    bytes(&annotate_rows_with_reverse(work, vec![navigation.clone()], reverse)?[0])?
                        .len()
                } else {
                    bytes(&navigation)?.len()
                };
                if navigation_size <= PREVIEW_BYTES.saturating_sub(preview_bytes) {
                    preview_bytes += navigation_size;
                    preview_items += 1;
                    navigation_only.insert(id.clone());
                    rows.push(navigation);
                }
            }
            continue;
        }
        preview_bytes += size;
        preview_items += 1;
        supplied.insert(id.clone());
        rows.push(record);
    }
    let selection_text = if entity_orientation {
        "Scoped ENTITY_SCOPE and service-related DOMAIN_ENTITY records, then retained selected-section dependencies and discovered entrypoint declarations. No arbitrary symbol sampling."
    } else {
        "Retained selected-section dependencies, then discovered entrypoint declarations. No arbitrary symbol sampling."
    };
    let mut discovery = json!({
        "section":section,"availableDependencyCount":total,"countsByKind":counts,
        "suppliedDependencyCount":supplied.len(),"deferredDependencyCount":total.saturating_sub(supplied.len()),
        "previewLimits":{"maxItems":PREVIEW_ITEMS,"maxBytes":PREVIEW_BYTES},
        "selection":selection_text,
        "nextRead":"Use sourceReferences/dependencyReferences with work expand, or a query with kind and symbolContains. Deferred facts are not absent or unsupported; read them before citing them.",
        "influenceCoverage":"Full captured Work influence is unchanged by this context preview."
    });
    if entity_orientation {
        discovery["navigationOnlyCount"] = json!(navigation_only.len());
        let entity_scope_references: Vec<_> = work
            .checked
            .dependencies
            .values()
            .filter(|dependency| dependency.kind == "ENTITY_SCOPE" && dependency.service == service)
            .filter(|scope| work.influence.contains_key(&scope.id))
            .filter_map(|scope| {
                work.handles
                    .iter()
                    .find(|(_, handle)| handle.kind == "DEPENDENCY" && handle.id == scope.id)
                    .map(|(reference, _)| reference.clone())
            })
            .collect();
        discovery["entityNavigation"] = json!({
            "scopeReferences":entity_scope_references,
            "deferredEntityQuery":{"query":{"kind":"DOMAIN_ENTITY","symbolContains":""}},
            "limitations":[
                "DOMAIN_ENTITY records preserve declared provenance but do not establish lifecycle, ownership, activation or runtime behavior.",
                "Underlying source and dependency references are navigation; expand and record their contents before citing them.",
                "A declaration larger than the Work page byte cap remains subject to the existing item budget and is reported as omitted by the page reader."
            ]
        });
    }
    rows.push(json!({"kind":"EVIDENCE_DISCOVERY","id":format!("evidence-index:{service}"),"record":discovery}));
    Ok(rows)
}

fn rows(work: &Work, selection: &Selection) -> Result<Vec<Value>, ClewError> {
    validate_selection(selection)?;
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
        let matches = work.checked.dependencies.values().filter(|dependency| {
            (query.kind == "*" || dependency.kind == query.kind)
                && dependency.symbol.contains(&query.symbol_contains)
        });
        match query.projection {
                QueryProjection::Raw => matches
                    .map(|dependency| {
                        json!({"kind":"DEPENDENCY","id":dependency.id,"record":dependency})
                    })
                    .collect(),
                QueryProjection::Navigation => {
                    let reverse = reverse_handles(work);
                    matches
                        .filter(|dependency| work.influence.contains_key(&dependency.id))
                        .map(|dependency| callable_navigation_row(dependency, &reverse))
                        .collect()
                }
            }
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
    if selection.references.is_empty()
        && selection.symbols.is_empty()
        && selection.query.is_none()
        && work.request.context_profile.as_deref() == Some(HTTP_API_CONTRACT_PROFILE)
    {
        let (contract_rows, _) = http_api_contract_preparation(work)?;
        let mut present: BTreeSet<_> = items
            .iter()
            .map(|item| {
                (
                    item["kind"].as_str().unwrap_or_default().to_owned(),
                    item["id"].as_str().unwrap_or_default().to_owned(),
                )
            })
            .collect();
        items.extend(contract_rows.into_iter().filter(|item| {
            present.insert((
                item["kind"].as_str().unwrap_or_default().to_owned(),
                item["id"].as_str().unwrap_or_default().to_owned(),
            ))
        }));
    }
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

fn callable_navigation_row(
    dependency: &Observation,
    reverse: &BTreeMap<(&str, &str), &str>,
) -> Value {
    let source_references: Vec<_> = dependency
        .source_ids
        .iter()
        .filter_map(|id| reverse.get(&("SOURCE", id.as_str())).copied())
        .collect();
    let exact_identity = dependency.normalized["symbolIdentity"]
        .as_str()
        .or_else(|| dependency.normalized["compilerCallableId"].as_str())
        .unwrap_or(&dependency.symbol);
    let source_tokens_present = dependency.normalized["sourceTokens"]
        .as_array()
        .is_some_and(|tokens| !tokens.is_empty());
    let mut row = json!({
        "kind":"CALLABLE_SUMMARY",
        "id":dependency.id,
        "referenceRoles":[],
        "record":{
            "identity":exact_identity,
            "symbol":dependency.symbol,
            "service":dependency.service,
            "scope":dependency.normalized["scope"],
            "name":dependency.normalized["name"],
            "owner":dependency.normalized["ownerIdentity"],
            "fullRecordReference":reverse.get(&("DEPENDENCY", dependency.id.as_str())).copied(),
            "sourceReferences":source_references,
            "authority":"NAVIGATION_ONLY",
            "provenance":{"dependencyId":dependency.id,"observationDigest":dependency.digest,"kind":dependency.kind},
            "bodyAvailability":{
                "capturedDeclarationTokensAvailable":source_tokens_present,
                "relatedSourceReferences":source_references,
                "relatedSourceRecords":"NAVIGATION_ONLY"
            },
        }
    });
    if !dependency.normalized["declarationKind"].is_null() {
        row["record"]["declarationKind"] = dependency.normalized["declarationKind"].clone();
    }
    if !dependency.normalized["syntaxKind"].is_null() {
        row["record"]["syntaxKind"] = dependency.normalized["syntaxKind"].clone();
    }
    row
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

fn reverse_handles(work: &Work) -> BTreeMap<(&str, &str), &str> {
    work.handles
        .iter()
        .map(|(key, handle)| ((handle.kind.as_str(), handle.id.as_str()), key.as_str()))
        .collect()
}

fn annotate_rows(work: &Work, items: Vec<Value>) -> Result<Vec<Value>, ClewError> {
    let reverse = reverse_handles(work);
    annotate_rows_with_reverse(work, items, &reverse)
}

fn annotate_rows_with_reverse<'a>(
    work: &Work,
    mut items: Vec<Value>,
    reverse: &BTreeMap<(&'a str, &'a str), &'a str>,
) -> Result<Vec<Value>, ClewError> {
    // Dynamic view/process facts outside this work subject are not supplied
    // or admitted as implicit influence of a service-only explanation.
    items.retain(|item| {
        item["kind"] != "DEPENDENCY"
            || item["id"]
                .as_str()
                .is_some_and(|id| work.influence.contains_key(id))
    });
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
        let entity_orientation_record = work.request.context_profile.as_deref()
            == Some(SECTION_ENTITIES_ORIENTATION_PROFILE)
            && matches!(
                item["record"]["kind"].as_str(),
                Some("DOMAIN_ENTITY" | "ENTITY_SCOPE")
            );
        if entity_orientation_record
            && let Some(ids) = item["record"]["normalized"]["dependencyIds"].as_array()
        {
            let mut references = item["dependencyReferences"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            references.extend(ids.iter().filter_map(|id| {
                reverse
                    .get(&("DEPENDENCY", id.as_str()?))
                    .map(|reference| json!(reference))
            }));
            item["dependencyReferences"] = json!(references);
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
    read_loaded_with_requested(repo, work, selection, None)
}

pub(super) fn read_loaded_with_requested(
    repo: &Repository,
    work: &Work,
    selection: Selection,
    requested_selection: Option<Selection>,
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
        requested_selection: requested_selection.filter(|requested| requested != &selection),
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

    fn entity_fixture(description_bytes: usize) -> Work {
        let mut work = fixture(1, false);
        work.request.entrypoint = Some("section-entities".into());
        work.request.context_profile = None;
        let source = super::super::model::Source {
            id: "retained-source".into(),
            service: "orders".into(),
            revision: "revision-test".into(),
            file: "src/order.rs".into(),
            start_line: 1,
            end_line: 2,
            text: "fn helper() {}".into(),
            text_digest: "text-digest".into(),
            evidence_digest: "evidence-digest".into(),
            authority: "CAPTURED_SOURCE".into(),
            occurrence: None,
            url: None,
        };
        work.checked.services.insert(
            "orders".into(),
            super::super::model::ServiceEvidence {
                schema: "codeclew-documentation-service-evidence/1.0".into(),
                service: "orders".into(),
                revision: "revision-test".into(),
                service_digest: "service-digest".into(),
                extractor: "test".into(),
                runtime_mode: "TEST".into(),
                coverage: "COMPLETE".into(),
                boundaries: Vec::new(),
                entrypoints: Vec::new(),
                observations: BTreeMap::new(),
                sources: BTreeMap::from([(source.id.clone(), source)]),
                contracts: BTreeMap::new(),
            },
        );

        let relation = json!({
            "service":"orders","kind":"created","origin":"human",
            "rationale":"The service declaration assigns creation responsibility.",
            "confidence":"declared","representations":["OrderRecord"],
            "dependencyIds":["orders:symbol:00000"]
        });
        let entity = json!({
            "schema":"codeclew-documentation-entity/1.0","id":"order","title":"Order",
            "description":"x".repeat(description_bytes),"relations":[relation],
            "relatedEntities":[],"limitations":["Runtime ownership is unverified."]
        });
        for (id, entity_json) in [
            ("entity:order", entity),
            (
                "entity:billing",
                json!({
                    "schema":"codeclew-documentation-entity/1.0","id":"billing","title":"Billing",
                    "description":"Unrelated declaration.",
                    "relations":[{"service":"billing","kind":"read","origin":"agent-proposal",
                        "rationale":"Belongs to another service.","confidence":"uncertain","representations":[],"dependencyIds":[]}],
                    "relatedEntities":[],"limitations":[]
                }),
            ),
        ] {
            let normalized = json!({
                "entity":entity_json,
                "dependencyIds":if id == "entity:order" { json!( ["orders:symbol:00000"] ) } else { json!([]) },
                "missingDependencies":[],"authority":"DECLARED_DOMAIN_ENTITY",
                "ownership":"Human declarations and agent proposals are separate; no runtime ownership proof"
            });
            let observation = Observation {
                id: id.into(),
                kind: "DOMAIN_ENTITY".into(),
                service: String::new(),
                symbol: normalized["entity"]["title"].as_str().unwrap().into(),
                digest: digest(&normalized).unwrap(),
                normalized,
                source_ids: vec!["retained-source".into()],
            };
            work.influence.insert(id.into(), observation.digest.clone());
            work.checked.dependencies.insert(id.into(), observation);
            let handle = if id == "entity:order" {
                "d-entity"
            } else {
                "d-unrelated"
            };
            work.handles.insert(
                handle.into(),
                Handle {
                    kind: "DEPENDENCY".into(),
                    id: id.into(),
                },
            );
        }
        let scope_normalized = json!({
            "dependencyIds":["entity:order"],
            "authority":"DECLARED_DOMAIN_ENTITY_SCOPE"
        });
        let scope = Observation {
            id: "entity-scope:orders".into(),
            kind: "ENTITY_SCOPE".into(),
            service: "orders".into(),
            symbol: "orders".into(),
            digest: digest(&scope_normalized).unwrap(),
            normalized: scope_normalized,
            source_ids: Vec::new(),
        };
        work.influence
            .insert(scope.id.clone(), scope.digest.clone());
        work.checked.dependencies.insert(scope.id.clone(), scope);
        work.handles.insert(
            "d-scope".into(),
            Handle {
                kind: "DEPENDENCY".into(),
                id: "entity-scope:orders".into(),
            },
        );
        normalize_section_profile(&work.subject, &mut work.request);
        work
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
    fn selection_modes_allow_empty_and_cursor_selections_but_reject_combinations() {
        let work = fixture(4, false);
        let valid = [
            json!({}),
            json!({"references":[],"symbols":[],"query":null}),
            json!({"references":["d1"]}),
            json!({"references":["d1"],"symbols":[],"query":null}),
            json!({"symbols":["Handler1"]}),
            json!({"references":[],"symbols":["Handler1"],"query":null}),
            json!({"query":{"kind":"SYMBOL","symbolContains":"Handler1"}}),
            json!({"query":{"kind":"SYMBOL","projection":"NAVIGATION"}}),
            json!({"references":[],"symbols":[],"query":{"kind":"SYMBOL"}}),
            json!({"cursor":"next-page"}),
            json!({"references":["d1"],"cursor":"next-page"}),
            json!({"symbols":["Handler1"],"cursor":"next-page"}),
            json!({"query":{"kind":"SYMBOL"},"cursor":"next-page"}),
        ];
        for value in valid {
            let selection: Selection = serde_json::from_value(value.clone()).unwrap();
            validate_selection(&selection)
                .unwrap_or_else(|error| panic!("valid selection rejected: {value}: {error}"));
            rows(&work, &selection)
                .unwrap_or_else(|error| panic!("valid selection failed rows: {value}: {error}"));
        }

        let mixed = [
            json!({"references":["d1"],"symbols":["Handler1"]}),
            json!({"references":["d1"],"query":{"kind":"SYMBOL"}}),
            json!({"symbols":["Handler1"],"query":{"kind":"SYMBOL"}}),
            json!({"references":["d1"],"symbols":["Handler1"],"query":{"kind":"SYMBOL"}}),
        ];
        for value in mixed {
            let selection: Selection = serde_json::from_value(value.clone()).unwrap();
            assert!(validate_selection(&selection).is_err(), "accepted {value}");
            assert!(rows(&work, &selection).is_err(), "rows accepted {value}");
        }
        let invalid_projection: Selection = serde_json::from_value(json!({
            "query":{"kind":"HTTP","projection":"NAVIGATION"}
        }))
        .unwrap();
        assert!(validate_selection(&invalid_projection).is_err());
    }

    #[test]
    fn symbol_navigation_bounds_giant_declarations_and_binds_its_own_cursor() {
        let temporary = tempfile::tempdir().unwrap();
        Repository::init(temporary.path(), "Architecture").unwrap();
        let repo = Repository::open(temporary.path()).unwrap();
        let mut work = fixture(3, false);
        work.id = "a".repeat(64);
        work.request.max_items = 1;
        work.request.max_bytes = 4096;
        let declaration = work
            .checked
            .dependencies
            .get_mut("orders:symbol:00000")
            .unwrap();
        declaration.symbol = "method:class:orders.HugeService#run()V".into();
        declaration.normalized = json!({
            "symbolIdentity":"method:class:orders.HugeService#run()V",
            "compilerCallableId":"orders.HugeService.run",
            "scope":":main",
            "name":"run",
            "ownerIdentity":"class:orders.HugeService",
            "declarationKind":"METHOD",
            "syntaxKind":"METHOD_DECLARATION",
            "sourceTokens":["public","void","run","(",")"],
            "classBodyTokens":"x".repeat(80_000),
            "documentation":{"events":[{"kind":"CALL","resolution":"COMPILER_EXACT","target":"method:class:orders.Other#target()V"}]}
        });
        declaration.digest = digest(&declaration.normalized).unwrap();
        work.influence
            .insert(declaration.id.clone(), declaration.digest.clone());

        let raw = Selection {
            query: Some(Query {
                kind: "SYMBOL".into(),
                symbol_contains: String::new(),
                projection: QueryProjection::Raw,
            }),
            ..Selection::default()
        };
        assert_eq!(
            serde_json::to_value(raw.query.as_ref().unwrap()).unwrap(),
            json!({"kind":"SYMBOL","symbolContains":""})
        );
        let raw_page = read_loaded(&repo, &work, raw.clone()).unwrap();
        assert_eq!(raw_page["items"], json!([]));
        assert_eq!(
            raw_page["omitted"][0]["reason"],
            "ITEM_EXCEEDS_WORK_BYTE_BUDGET"
        );
        let raw_cursor = raw_page["nextCursor"].as_str().unwrap().to_owned();

        let navigation = Selection {
            query: Some(Query {
                kind: "SYMBOL".into(),
                symbol_contains: String::new(),
                projection: QueryProjection::Navigation,
            }),
            ..Selection::default()
        };
        let navigation_page = read_loaded(&repo, &work, navigation.clone()).unwrap();
        assert_eq!(navigation_page["items"].as_array().unwrap().len(), 1);
        assert!(navigation_page["omitted"].as_array().unwrap().is_empty());
        assert!(navigation_page["items"][0].get("reference").is_none());
        assert_eq!(
            navigation_page["items"][0]["record"]["fullRecordReference"],
            "d0"
        );
        assert_eq!(
            navigation_page["items"][0]["record"]["sourceReferences"],
            json!(["s1"])
        );
        assert_eq!(
            navigation_page["items"][0]["record"]["identity"],
            "method:class:orders.HugeService#run()V"
        );
        assert_eq!(
            navigation_page["items"][0]["record"]["bodyAvailability"]["capturedDeclarationTokensAvailable"],
            true
        );
        assert_eq!(
            navigation_page["items"][0]["record"]["authority"],
            "NAVIGATION_ONLY"
        );
        assert_eq!(
            navigation_page["items"][0]["record"]["declarationKind"],
            "METHOD"
        );
        assert_eq!(
            navigation_page["items"][0]["record"]["syntaxKind"],
            "METHOD_DECLARATION"
        );
        assert_eq!(
            read_state(&repo, &work.id)
                .unwrap()
                .receipts
                .values()
                .last()
                .unwrap()
                .supplied,
            Vec::<String>::new()
        );

        let mut empty_tokens_work = work.clone();
        empty_tokens_work.id = "c".repeat(64);
        let empty_tokens_declaration = empty_tokens_work
            .checked
            .dependencies
            .get_mut("orders:symbol:00000")
            .unwrap();
        empty_tokens_declaration.normalized["sourceTokens"] = json!([]);
        empty_tokens_declaration.digest = digest(&empty_tokens_declaration.normalized).unwrap();
        empty_tokens_work.influence.insert(
            empty_tokens_declaration.id.clone(),
            empty_tokens_declaration.digest.clone(),
        );
        let empty_token_page = read_loaded(&repo, &empty_tokens_work, navigation.clone()).unwrap();
        assert_eq!(
            empty_token_page["items"][0]["record"]["bodyAvailability"]["capturedDeclarationTokensAvailable"],
            false
        );

        let navigation_cursor = navigation_page["nextCursor"].as_str().unwrap().to_owned();
        let mut continue_navigation = navigation.clone();
        continue_navigation.cursor = Some(navigation_cursor.clone());
        let next = read_loaded(&repo, &work, continue_navigation).unwrap();
        assert_eq!(next["items"][0]["id"], "orders:symbol:00001");
        let mut mix_raw_cursor = navigation.clone();
        mix_raw_cursor.cursor = Some(raw_cursor);
        assert!(read_loaded(&repo, &work, mix_raw_cursor).is_err());
        let mut mix_navigation_cursor = raw;
        mix_navigation_cursor.cursor = Some(navigation_cursor);
        assert!(read_loaded(&repo, &work, mix_navigation_cursor).is_err());
    }

    #[test]
    fn dependency_query_matches_captured_method_symbols_and_wildcards_kinds() {
        let mut work = fixture(2, false);
        let method_id = "orders:symbol:00001";
        let method = work.checked.dependencies.get_mut(method_id).unwrap();
        method.symbol = "OrderService.helperMethod".into();
        method.normalized = json!({"name":"OrderService","methods":[{"name":"helperMethod"}]});
        method.digest = digest(&method.normalized).unwrap();
        work.influence
            .insert(method_id.into(), method.digest.clone());

        let http_id = "orders:http:helper";
        let http_record = Observation {
            id: http_id.into(),
            kind: "HTTP".into(),
            service: "orders".into(),
            symbol: "helper endpoint".into(),
            digest: digest(&json!({"path":"/helper"})).unwrap(),
            normalized: json!({"path":"/helper"}),
            source_ids: vec!["retained-source".into()],
        };
        work.influence
            .insert(http_id.into(), http_record.digest.clone());
        work.checked
            .dependencies
            .insert(http_id.into(), http_record);

        let matching = rows(
            &work,
            &Selection {
                query: Some(Query {
                    kind: "SYMBOL".into(),
                    symbol_contains: "helper".into(),
                    projection: QueryProjection::Raw,
                }),
                ..Selection::default()
            },
        )
        .unwrap();
        assert_eq!(matching.len(), 1);
        assert_eq!(matching[0]["id"], method_id);
        assert_eq!(matching[0]["record"]["symbol"], "OrderService.helperMethod");

        let page_row_kind_confusion = rows(
            &work,
            &Selection {
                query: Some(Query {
                    kind: "SOURCE".into(),
                    symbol_contains: "helper".into(),
                    projection: QueryProjection::Raw,
                }),
                ..Selection::default()
            },
        )
        .unwrap();
        assert!(page_row_kind_confusion.is_empty());

        let no_match = rows(
            &work,
            &Selection {
                query: Some(Query {
                    kind: "SYMBOL".into(),
                    symbol_contains: "missing".into(),
                    projection: QueryProjection::Raw,
                }),
                ..Selection::default()
            },
        )
        .unwrap();
        assert!(no_match.is_empty());

        let wildcard = rows(
            &work,
            &Selection {
                query: Some(Query {
                    kind: "*".into(),
                    symbol_contains: "helper".into(),
                    projection: QueryProjection::Raw,
                }),
                ..Selection::default()
            },
        )
        .unwrap();
        assert_eq!(wildcard.len(), 2);
        assert_eq!(
            wildcard
                .iter()
                .map(|row| row["id"].as_str().unwrap())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([method_id, http_id])
        );
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

        let mut legacy_entity = entity_fixture(0);
        legacy_entity.request.context_profile = Some(SECTION_ORIENTATION_PROFILE.into());
        let old_entity_id =
            digest(&StoredWork::from_runtime(&legacy_entity, "capture".into())).unwrap();
        let error =
            validate_context_profile(&legacy_entity.subject, &legacy_entity.request).unwrap_err();
        assert!(error.message.contains("retained snapshot"));

        legacy_entity.request.context_profile = None;
        assert!(validate_context_profile(&legacy_entity.subject, &legacy_entity.request).is_err());
        normalize_section_profile(&legacy_entity.subject, &mut legacy_entity.request);
        assert_eq!(
            legacy_entity.request.context_profile.as_deref(),
            Some(SECTION_ENTITIES_ORIENTATION_PROFILE)
        );
        validate_context_profile(&legacy_entity.subject, &legacy_entity.request).unwrap();
        assert_ne!(
            old_entity_id,
            digest(&StoredWork::from_runtime(&legacy_entity, "capture".into())).unwrap()
        );
    }

    #[test]
    fn entity_projection_keeps_scope_provenance_and_expandable_proof_handles() {
        let work = entity_fixture(32);
        let initial = rows(&work, &Selection::default()).unwrap();
        let supplied: Vec<_> = initial
            .iter()
            .filter(|row| {
                matches!(
                    row["kind"].as_str(),
                    Some("DEPENDENCY" | "ENTITY_NAVIGATION")
                )
            })
            .collect();
        assert!(supplied.len() <= 8);
        assert!(bytes(&initial).unwrap().len() < 8192);
        assert!(
            initial
                .iter()
                .any(|row| { row["kind"] == "DEPENDENCY" && row["id"] == "entity-scope:orders" })
        );
        assert!(
            initial
                .iter()
                .any(|row| row["kind"] == "DEPENDENCY" && row["id"] == "entity:order")
        );
        assert!(!initial.iter().any(|row| row["id"] == "entity:billing"));

        let scope = initial
            .iter()
            .find(|row| row["id"] == "entity-scope:orders")
            .unwrap();
        assert_eq!(scope["reference"], "d-scope");
        assert_eq!(scope["dependencyReferences"], json!(["d-entity"]));
        let entity = initial
            .iter()
            .find(|row| row["id"] == "entity:order")
            .unwrap();
        assert_eq!(
            entity["record"]["normalized"]["entity"]["relations"][0]["origin"],
            "human"
        );
        assert_eq!(
            entity["record"]["normalized"]["entity"]["relations"][0]["confidence"],
            "declared"
        );
        assert!(
            entity["record"]["normalized"]["ownership"]
                .as_str()
                .unwrap()
                .contains("no runtime ownership proof")
        );
        assert_eq!(entity["reference"], "d-entity");
        assert_eq!(entity["dependencyReferences"], json!(["d0"]));
        assert_eq!(entity["sourceReferences"], json!(["s1"]));

        let discovery = initial
            .iter()
            .find(|row| row["kind"] == "EVIDENCE_DISCOVERY")
            .unwrap();
        assert_eq!(
            discovery["record"]["entityNavigation"]["scopeReferences"],
            json!(["d-scope"])
        );
        assert!(
            !discovery["record"]["entityNavigation"]["limitations"]
                .as_array()
                .unwrap()
                .is_empty()
        );

        let expanded_scope = rows(
            &work,
            &Selection {
                references: vec!["d-scope".into()],
                ..Selection::default()
            },
        )
        .unwrap();
        let expanded_scope = expanded_scope
            .iter()
            .find(|row| row["id"] == "entity-scope:orders")
            .unwrap();
        assert_eq!(expanded_scope["dependencyReferences"], json!(["d-entity"]));

        let expanded_entity = rows(
            &work,
            &Selection {
                references: vec!["d-entity".into()],
                ..Selection::default()
            },
        )
        .unwrap();
        let expanded_entity = expanded_entity
            .iter()
            .find(|row| row["id"] == "entity:order")
            .unwrap();
        assert_eq!(expanded_entity["dependencyReferences"], json!(["d0"]));
        assert_eq!(expanded_entity["sourceReferences"], json!(["s1"]));
        let expanded_proof = rows(
            &work,
            &Selection {
                references: vec!["d0".into()],
                ..Selection::default()
            },
        )
        .unwrap();
        assert!(
            expanded_proof
                .iter()
                .any(|row| row["id"] == "orders:symbol:00000")
        );
        let expanded_source = rows(
            &work,
            &Selection {
                references: vec!["s1".into()],
                ..Selection::default()
            },
        )
        .unwrap();
        assert!(expanded_source.iter().any(|row| row["kind"] == "SOURCE"));
    }

    #[test]
    fn oversized_entity_is_navigation_only_but_registered_query_expands_full_record() {
        let work = entity_fixture(8192);
        let initial = rows(&work, &Selection::default()).unwrap();
        let navigation = initial
            .iter()
            .find(|row| row["kind"] == "ENTITY_NAVIGATION")
            .unwrap();
        assert!(navigation.get("reference").is_none());
        assert_eq!(navigation["dependencyReferences"], json!(["d-entity"]));
        assert!(navigation["record"]["normalized"]["entity"].is_null());
        assert!(bytes(&initial).unwrap().len() < 8192);
        assert!(
            initial
                .iter()
                .filter(|row| {
                    matches!(
                        row["kind"].as_str(),
                        Some("DEPENDENCY" | "ENTITY_NAVIGATION")
                    )
                })
                .count()
                <= 8
        );

        let expanded = rows(
            &work,
            &Selection {
                query: Some(Query {
                    kind: "DOMAIN_ENTITY".into(),
                    symbol_contains: "Order".into(),
                    projection: QueryProjection::Raw,
                }),
                ..Selection::default()
            },
        )
        .unwrap();
        assert_eq!(expanded.len(), 1);
        assert_eq!(
            expanded[0]["record"]["normalized"]["entity"]["description"]
                .as_str()
                .unwrap()
                .len(),
            8192
        );
        assert!(bytes(&expanded[0]).unwrap().len() > 8192);
        assert!(bytes(&expanded).unwrap().len() < work.request.max_bytes);
    }

    #[test]
    fn other_section_profile_keeps_entity_reference_and_cursor_projection_unchanged() {
        let mut work = entity_fixture(32);
        work.request.entrypoint = Some("section-responsibilities".into());
        work.request.context_profile = Some(SECTION_ORIENTATION_PROFILE.into());
        work.handles.insert(
            "section-entities-ref".into(),
            Handle {
                kind: "SECTION".into(),
                id: "section-entities".into(),
            },
        );
        let no_cursor = rows(
            &work,
            &Selection {
                references: vec!["section-entities-ref".into()],
                ..Selection::default()
            },
        )
        .unwrap();
        let with_cursor = rows(
            &work,
            &Selection {
                references: vec!["section-entities-ref".into()],
                cursor: Some("cursor-1".into()),
                ..Selection::default()
            },
        )
        .unwrap();
        assert_eq!(no_cursor, with_cursor);
        assert!(
            no_cursor
                .iter()
                .all(|row| row["kind"] != "ENTITY_NAVIGATION")
        );
        let discovery = no_cursor
            .iter()
            .find(|row| row["kind"] == "EVIDENCE_DISCOVERY")
            .unwrap();
        assert!(discovery["record"].get("entityNavigation").is_none());

        let entity_query = rows(
            &work,
            &Selection {
                query: Some(Query {
                    kind: "DOMAIN_ENTITY".into(),
                    symbol_contains: "Order".into(),
                    projection: QueryProjection::Raw,
                }),
                ..Selection::default()
            },
        )
        .unwrap();
        assert_eq!(entity_query.len(), 1);
        assert!(entity_query[0].get("dependencyReferences").is_none());
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
                    projection: QueryProjection::Raw,
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

#[cfg(test)]
mod api_contract_tests {
    use super::*;

    const SCOPE: &str = ":main";
    const DESCRIPTOR: &str = "(Lorders/Request;I[Z)Lorders/Response;";

    fn add_source(work: &mut Work, id: &str, service: &str, text: String) {
        let source = Source {
            id: id.into(),
            service: service.into(),
            revision: "revision-test".into(),
            file: format!("src/{id}.java"),
            start_line: 1,
            end_line: text.lines().count().max(1) as u64,
            text_digest: crate::canonical::hash_bytes(text.as_bytes()),
            text,
            evidence_digest: "evidence-test".into(),
            authority: "CAPTURED_SOURCE".into(),
            occurrence: None,
            url: None,
        };
        if let Some(evidence) = work.checked.services.get_mut(service) {
            evidence.sources.insert(id.into(), source);
        }
        let reference = format!("source-{}", work.handles.len());
        work.handles.insert(
            reference,
            Handle {
                kind: "SOURCE".into(),
                id: id.into(),
            },
        );
    }

    fn add_observation(
        work: &mut Work,
        id: &str,
        service: &str,
        kind: &str,
        symbol: &str,
        normalized: Value,
        source_ids: Vec<String>,
    ) {
        let fact_digest = digest(&normalized).unwrap();
        let observation = Observation {
            id: id.into(),
            kind: kind.into(),
            service: service.into(),
            symbol: symbol.into(),
            normalized,
            digest: fact_digest.clone(),
            source_ids,
        };
        if let Some(evidence) = work.checked.services.get_mut(service) {
            evidence.observations.insert(id.into(), observation.clone());
        }
        work.checked.dependencies.insert(id.into(), observation);
        work.influence.insert(id.into(), fact_digest);
        let reference = format!("dependency-{}", work.handles.len());
        work.handles.insert(
            reference,
            Handle {
                kind: "DEPENDENCY".into(),
                id: id.into(),
            },
        );
    }

    fn api_fixture(descriptor: &str, types: &[&str], request_source_bytes: usize) -> Work {
        let mut work = Work {
            schema: WORK_SCHEMA.into(),
            id: "work".into(),
            subject: "service:orders".into(),
            request: Request {
                schema: "codeclew-documentation-work-request/1.0".into(),
                audience: "Maintainers".into(),
                documentation_language: Some("en".into()),
                entrypoint: Some("entry-http".into()),
                context_profile: None,
                max_items: 100,
                max_bytes: 49152,
                external_inputs: Vec::new(),
            },
            checked: Check {
                schema: "codeclew-documentation-check/1.0".into(),
                input_digest: "input".into(),
                context_digest: "context".into(),
                services: BTreeMap::new(),
                unresolved: BTreeMap::new(),
                interactions: BTreeMap::new(),
                scenarios: BTreeMap::new(),
                dependencies: BTreeMap::new(),
                source_inputs: None,
                composition: None,
            },
            snapshot: Some("snapshot-test".into()),
            retained: None,
            external_inputs: BTreeMap::new(),
            handles: BTreeMap::new(),
            influence: BTreeMap::new(),
            obligations: Vec::new(),
            review_reasons: Vec::new(),
        };
        work.checked.services.insert(
            "orders".into(),
            ServiceEvidence {
                schema: "codeclew-documentation-service-evidence/1.0".into(),
                service: "orders".into(),
                revision: "revision-test".into(),
                service_digest: "service-digest".into(),
                extractor: EXTRACTOR.into(),
                runtime_mode: "BUILD".into(),
                coverage: "COMPLETE".into(),
                boundaries: Vec::new(),
                entrypoints: Vec::new(),
                observations: BTreeMap::new(),
                sources: BTreeMap::new(),
                contracts: BTreeMap::new(),
            },
        );

        let endpoint_source = "@PostMapping(\"/orders\") Response handle(Request request)";
        add_source(
            &mut work,
            "endpoint-source",
            "orders",
            endpoint_source.into(),
        );
        let endpoint_symbol = format!("method:class:orders.Controller#handle{descriptor}");
        add_observation(
            &mut work,
            "endpoint-declaration",
            "orders",
            "SYMBOL",
            &endpoint_symbol,
            json!({
                "schema":JAVA_COMPILER_FACT_SCHEMA,
                "declarationKind":"METHOD",
                "symbolIdentity":endpoint_symbol,
                "ownerIdentity":"class:orders.Controller",
                "name":"handle",
                "resolution":"COMPILER_EXACT",
                "scope":SCOPE,
                "jvmDescriptor":descriptor,
                "documentation":{"events":[],"parameterTypes":["ignored.Generic<orders.Payload>"]}
            }),
            vec!["endpoint-source".into()],
        );
        add_observation(
            &mut work,
            "endpoint-route",
            "orders",
            "ENTRYPOINT",
            &endpoint_symbol,
            json!({"kind":"HTTP_ENDPOINT","scope":SCOPE}),
            vec!["endpoint-source".into()],
        );
        work.checked.services.get_mut("orders").unwrap().entrypoints = vec![Entrypoint {
            id: "entry-http".into(),
            service: "orders".into(),
            symbol: endpoint_symbol,
            kind: "HTTP_ENDPOINT".into(),
            trigger: json!({"methods":["POST"],"paths":["/orders"]}),
            source_ids: vec!["endpoint-source".into()],
            dependency_ids: vec!["endpoint-declaration".into(), "endpoint-route".into()],
            boundaries: Vec::new(),
        }];

        for identity in types {
            let simple_name = identity.rsplit('.').next().unwrap_or(identity);
            let mut text = if simple_name == "Request" {
                "class Request extends BaseRequest { String name; }".to_owned()
            } else {
                format!("class {} {{}}", simple_name)
            };
            if simple_name == "Request" && request_source_bytes > text.len() {
                text.push_str(&" ".repeat(request_source_bytes - text.len()));
            }
            let id = format!("dto-{}", work.checked.dependencies.len());
            let source_id = format!("source-{id}");
            add_source(&mut work, &source_id, "orders", text);
            add_observation(
                &mut work,
                &id,
                "orders",
                "SYMBOL",
                identity,
                json!({
                    "schema":JAVA_COMPILER_FACT_SCHEMA,
                    "declarationKind":"CLASS",
                    "symbolIdentity":identity,
                    "qualifiedName":identity.strip_prefix("class:").unwrap_or(identity),
                    "scope":SCOPE,
                }),
                vec![source_id],
            );
        }
        add_observation(
            &mut work,
            "base-request-type",
            "orders",
            "SYMBOL",
            "class:orders.BaseRequest",
            json!({
                "schema":JAVA_COMPILER_FACT_SCHEMA,
                "declarationKind":"CLASS",
                "symbolIdentity":"class:orders.BaseRequest",
                "qualifiedName":"orders.BaseRequest",
                "scope":SCOPE,
            }),
            Vec::new(),
        );
        add_observation(
            &mut work,
            "request-getter-member",
            "orders",
            "SYMBOL",
            "method:class:orders.Request#getName()Ljava/lang/String;",
            json!({
                "schema":JAVA_COMPILER_FACT_SCHEMA,
                "declarationKind":"METHOD",
                "symbolIdentity":"method:class:orders.Request#getName()Ljava/lang/String;",
                "ownerIdentity":"class:orders.Request",
                "scope":SCOPE,
            }),
            Vec::new(),
        );
        work
    }

    fn prepared_api_work(descriptor: &str, types: &[&str], request_source_bytes: usize) -> Work {
        let mut work = api_fixture(descriptor, types, request_source_bytes);
        normalize_http_api_contract_profile(&work.subject, &mut work.request, &work.checked);
        validate_http_api_contract_profile(&work.subject, &work.request, &work.checked).unwrap();
        let (_, obligation) = http_api_contract_preparation(&work).unwrap();
        work.obligations.push(obligation);
        work
    }

    fn seal_work_identity(work: &mut Work) {
        let mut stored = StoredWork::from_runtime(work, "snapshot-test".into());
        stored.id = digest(&stored).unwrap()[7..].into();
        work.id = stored.id;
    }

    fn direct_dependency_symbols(rows: &[Value]) -> BTreeSet<String> {
        rows.iter()
            .filter(|row| row["kind"] == "DEPENDENCY")
            .filter_map(|row| row["record"]["symbol"].as_str().map(str::to_owned))
            .collect()
    }

    #[test]
    fn descriptor_parser_validates_full_signature_and_keeps_direct_object_identities() {
        assert_eq!(
            descriptor_object_types("([Lpkg/Outer$Inner;I[[ZJ)Lpkg/Reply$Data;"),
            Ok(BTreeSet::from([
                "class:pkg.Outer$Inner".into(),
                "class:pkg.Reply$Data".into(),
            ]))
        );
        assert_eq!(descriptor_object_types("()V"), Ok(BTreeSet::new()));
        assert_eq!(descriptor_object_types("([IJD)Z"), Ok(BTreeSet::new()));
        assert_eq!(
            descriptor_object_types(&format!("({})V", "I".repeat(255))),
            Ok(BTreeSet::new())
        );
        assert_eq!(
            descriptor_object_types(&format!("({})V", "I".repeat(256))),
            Err(())
        );
        assert_eq!(
            descriptor_object_types(&format!("({}I)V", "[".repeat(255))),
            Ok(BTreeSet::new())
        );
        assert_eq!(
            descriptor_object_types(&format!("({}I)V", "[".repeat(256))),
            Err(())
        );
        for malformed in [
            "(Lpkg/Request;)Vsuffix",
            "(Lpkg/Request;)Lpkg/Reply",
            "(Lpkg/Request;)",
            "(V)V",
            "()[V",
            "()L;",
            "(I)Lpkg/Reply;;",
            "(Lpkg//Request;)V",
        ] {
            assert_eq!(descriptor_object_types(malformed), Err(()), "{malformed}");
        }
    }

    #[test]
    fn default_http_work_adds_direct_type_declarations_and_sources_without_member_or_inherited_fanout()
     {
        let work = prepared_api_work(
            DESCRIPTOR,
            &["class:orders.Request", "class:orders.Response"],
            64,
        );
        assert_eq!(
            work.request.context_profile.as_deref(),
            Some(HTTP_API_CONTRACT_PROFILE)
        );
        let rows = rows(&work, &Selection::default()).unwrap();
        let symbols = direct_dependency_symbols(&rows);
        assert!(symbols.contains("class:orders.Request"));
        assert!(symbols.contains("class:orders.Response"));
        assert!(!symbols.contains("class:orders.BaseRequest"));
        assert!(!symbols.contains("method:class:orders.Request#getName()Ljava/lang/String;"));
        let source_ids: BTreeSet<_> = rows
            .iter()
            .filter(|row| row["kind"] == "SOURCE")
            .filter_map(|row| row["id"].as_str().map(str::to_owned))
            .collect();
        assert!(source_ids.contains("source-dto-2"));
        assert!(source_ids.contains("source-dto-3"));
        let obligation = rows
            .iter()
            .find(|row| {
                row["kind"] == "OBLIGATION"
                    && row["record"]["kind"] == "HTTP_API_CONTRACT_PREPARATION"
            })
            .unwrap();
        assert_eq!(obligation["record"]["directDescriptorTypeCount"], 2);
        assert_eq!(obligation["record"]["selectedDeclarationCount"], 2);
        assert!(
            obligation["record"]["limitations"]
                .as_array()
                .unwrap()
                .iter()
                .any(|limitation| limitation["code"] == "DIRECT_DECLARED_TYPES_ONLY")
        );
        assert!(
            obligation["record"]["limitations"][0]["detail"]
                .as_str()
                .unwrap()
                .contains("inherited fields")
        );
    }

    #[test]
    fn exact_type_matching_reports_missing_ambiguous_and_out_of_scope_candidates() {
        let mut wrong_scope = prepared_api_work(
            DESCRIPTOR,
            &["class:orders.Request", "class:orders.Response"],
            64,
        );
        wrong_scope
            .checked
            .dependencies
            .get_mut("dto-2")
            .unwrap()
            .normalized["scope"] = json!(":test");
        let (rows, obligation) = http_api_contract_preparation(&wrong_scope).unwrap();
        assert!(!direct_dependency_symbols(&rows).contains("class:orders.Request"));
        assert!(
            obligation["limitations"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["code"] == "DIRECT_TYPE_DECLARATION_MISSING")
        );

        let mut inconsistent_type = prepared_api_work(
            DESCRIPTOR,
            &["class:orders.Request", "class:orders.Response"],
            64,
        );
        inconsistent_type
            .checked
            .dependencies
            .get_mut("dto-2")
            .unwrap()
            .symbol = "class:orders.Different".into();
        let (rows, obligation) = http_api_contract_preparation(&inconsistent_type).unwrap();
        assert!(!direct_dependency_symbols(&rows).contains("class:orders.Request"));
        assert!(
            obligation["limitations"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["code"] == "DIRECT_TYPE_METADATA_UNSUPPORTED")
        );

        let mut no_endpoint_scope = prepared_api_work(
            DESCRIPTOR,
            &["class:orders.Request", "class:orders.Response"],
            64,
        );
        no_endpoint_scope
            .checked
            .dependencies
            .get_mut("endpoint-declaration")
            .unwrap()
            .normalized["scope"] = Value::Null;
        let (rows, obligation) = http_api_contract_preparation(&no_endpoint_scope).unwrap();
        assert!(rows.is_empty());
        assert!(
            obligation["limitations"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["code"] == "JAVA_ENDPOINT_COMPILATION_SCOPE_UNAVAILABLE")
        );

        let mut outside_influence = prepared_api_work(
            DESCRIPTOR,
            &["class:orders.Request", "class:orders.Response"],
            64,
        );
        outside_influence.influence.remove("dto-2");
        let (rows, _) = http_api_contract_preparation(&outside_influence).unwrap();
        assert!(!direct_dependency_symbols(&rows).contains("class:orders.Request"));

        let mut other_service = prepared_api_work(DESCRIPTOR, &["class:orders.Request"], 64);
        add_observation(
            &mut other_service,
            "inventory-response",
            "inventory",
            "SYMBOL",
            "class:orders.Response",
            json!({
                "schema":JAVA_COMPILER_FACT_SCHEMA,
                "declarationKind":"CLASS",
                "symbolIdentity":"class:orders.Response",
                "scope":SCOPE,
            }),
            Vec::new(),
        );
        let (rows, _) = http_api_contract_preparation(&other_service).unwrap();
        assert!(!direct_dependency_symbols(&rows).contains("class:orders.Response"));

        let missing = prepared_api_work(DESCRIPTOR, &["class:orders.Request"], 64);
        let (rows, obligation) = http_api_contract_preparation(&missing).unwrap();
        assert!(direct_dependency_symbols(&rows).contains("class:orders.Request"));
        assert!(
            obligation["limitations"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["code"] == "DIRECT_TYPE_DECLARATION_MISSING"
                    && item["symbolIdentity"] == "class:orders.Response")
        );
        assert_eq!(obligation["directDescriptorTypeCount"], 2);
        assert_eq!(obligation["selectedDeclarationCount"], 1);

        let mut ambiguous = prepared_api_work(
            DESCRIPTOR,
            &["class:orders.Request", "class:orders.Response"],
            64,
        );
        let mut duplicate = ambiguous.checked.dependencies["dto-2"].clone();
        duplicate.id = "duplicate-request".into();
        duplicate.digest = digest(&duplicate.normalized).unwrap();
        ambiguous
            .checked
            .services
            .get_mut("orders")
            .unwrap()
            .observations
            .insert(duplicate.id.clone(), duplicate.clone());
        ambiguous
            .checked
            .dependencies
            .insert(duplicate.id.clone(), duplicate.clone());
        ambiguous
            .influence
            .insert(duplicate.id.clone(), duplicate.digest.clone());
        let (rows, obligation) = http_api_contract_preparation(&ambiguous).unwrap();
        assert!(!direct_dependency_symbols(&rows).contains("class:orders.Request"));
        assert!(direct_dependency_symbols(&rows).contains("class:orders.Response"));
        assert!(
            obligation["limitations"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["code"] == "DIRECT_TYPE_DECLARATION_AMBIGUOUS")
        );
    }

    #[test]
    fn type_bound_defers_all_and_generic_erasure_does_not_seed_payload_types() {
        let identities: Vec<_> = (0..9)
            .map(|index| format!("class:orders.Type{index}"))
            .collect();
        let refs: Vec<_> = identities.iter().map(String::as_str).collect();
        let descriptor = format!(
            "({})V",
            (0..9)
                .map(|index| format!("Lorders/Type{index};"))
                .collect::<String>()
        );
        let over_bound = prepared_api_work(&descriptor, &refs, 32);
        let (rows, obligation) = http_api_contract_preparation(&over_bound).unwrap();
        assert!(rows.is_empty());
        assert_eq!(obligation["directDescriptorTypeCount"], 9);
        assert_eq!(obligation["selectedDeclarationCount"], 0);
        assert!(
            obligation["limitations"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["code"] == "DIRECT_TYPE_BOUND_DEFERRED")
        );

        let eight_refs: Vec<_> = identities[..8].iter().map(String::as_str).collect();
        let eight_descriptor = format!(
            "({})V",
            (0..8)
                .map(|index| format!("Lorders/Type{index};"))
                .collect::<String>()
        );
        let at_bound = prepared_api_work(&eight_descriptor, &eight_refs, 32);
        let (rows, obligation) = http_api_contract_preparation(&at_bound).unwrap();
        assert_eq!(direct_dependency_symbols(&rows).len(), 8);
        assert_eq!(obligation["selectedDeclarationCount"], 8);

        let erased = prepared_api_work(
            "(Lframework/ResponseEntity;)Lframework/ResponseEntity;",
            &["class:framework.ResponseEntity", "class:orders.Payload"],
            32,
        );
        let (rows, obligation) = http_api_contract_preparation(&erased).unwrap();
        assert_eq!(
            direct_dependency_symbols(&rows),
            BTreeSet::from(["class:framework.ResponseEntity".into()])
        );
        assert_eq!(obligation["directDescriptorTypeCount"], 1);
    }

    #[test]
    fn syntax_only_inconsistent_identity_and_non_http_roots_keep_legacy_membership() {
        let mut syntax = api_fixture(DESCRIPTOR, &["class:orders.Request"], 32);
        syntax.checked.services.get_mut("orders").unwrap().extractor =
            super::super::model::SOURCE_EXTRACTOR.into();
        normalize_http_api_contract_profile(&syntax.subject, &mut syntax.request, &syntax.checked);
        assert!(syntax.request.context_profile.is_none());

        let mut inconsistent = api_fixture(DESCRIPTOR, &["class:orders.Request"], 32);
        inconsistent
            .checked
            .dependencies
            .get_mut("endpoint-declaration")
            .unwrap()
            .normalized["symbolIdentity"] = json!("method:class:orders.Other#handle()V");
        normalize_http_api_contract_profile(
            &inconsistent.subject,
            &mut inconsistent.request,
            &inconsistent.checked,
        );
        assert!(inconsistent.request.context_profile.is_none());

        let mut unrelated = api_fixture(DESCRIPTOR, &["class:orders.Request"], 32);
        unrelated
            .checked
            .services
            .get_mut("orders")
            .unwrap()
            .entrypoints[0]
            .kind = "KAFKA_LISTENER".into();
        normalize_http_api_contract_profile(
            &unrelated.subject,
            &mut unrelated.request,
            &unrelated.checked,
        );
        assert!(unrelated.request.context_profile.is_none());
        assert!(
            !direct_dependency_symbols(&rows(&unrelated, &Selection::default()).unwrap())
                .contains("class:orders.Request")
        );
    }

    #[test]
    fn new_policy_identity_and_read_receipts_do_not_reinterpret_legacy_work() {
        let mut legacy = api_fixture(
            DESCRIPTOR,
            &["class:orders.Request", "class:orders.Response"],
            32,
        );
        seal_work_identity(&mut legacy);
        let legacy_id = legacy.id.clone();
        let mut fresh = legacy.clone();
        normalize_http_api_contract_profile(&fresh.subject, &mut fresh.request, &fresh.checked);
        let (_, obligation) = http_api_contract_preparation(&fresh).unwrap();
        fresh.obligations.push(obligation);
        seal_work_identity(&mut fresh);
        assert!(legacy.request.context_profile.is_none());
        assert_eq!(
            fresh.request.context_profile.as_deref(),
            Some(HTTP_API_CONTRACT_PROFILE)
        );
        assert_ne!(legacy_id, fresh.id);

        let temp = tempfile::tempdir().unwrap();
        Repository::init(temp.path(), "API contract receipt isolation").unwrap();
        let repo = Repository::open(temp.path()).unwrap();
        let legacy_page = read_loaded(&repo, &legacy, Selection::default()).unwrap();
        let old_state_before = read_state(&repo, &legacy.id).unwrap();
        let fresh_page = read_loaded(&repo, &fresh, Selection::default()).unwrap();
        let old_state_after = read_state(&repo, &legacy.id).unwrap();
        let new_state = read_state(&repo, &fresh.id).unwrap();
        let legacy_symbols = direct_dependency_symbols(legacy_page["items"].as_array().unwrap());
        assert!(!legacy_symbols.contains("class:orders.Request"));
        assert!(!legacy_symbols.contains("class:orders.Response"));
        assert!(
            direct_dependency_symbols(fresh_page["items"].as_array().unwrap())
                .contains("class:orders.Request")
        );
        assert_eq!(
            bytes(&old_state_before.receipts).unwrap(),
            bytes(&old_state_after.receipts).unwrap()
        );
        assert_ne!(
            old_state_after
                .receipts
                .values()
                .next()
                .unwrap()
                .membership_digest,
            new_state
                .receipts
                .values()
                .next()
                .unwrap()
                .membership_digest
        );

        let endpoint_reference = fresh
            .handles
            .iter()
            .find(|(_, handle)| handle.kind == "DEPENDENCY" && handle.id == "endpoint-declaration")
            .map(|(reference, _)| reference.clone())
            .unwrap();
        let explicit = rows(
            &fresh,
            &Selection {
                references: vec![endpoint_reference],
                ..Selection::default()
            },
        )
        .unwrap();
        assert!(!direct_dependency_symbols(&explicit).contains("class:orders.Request"));
    }

    #[test]
    fn oversized_sources_remain_part_readable_while_oversized_declarations_block_completeness() {
        let mut source_work = prepared_api_work(
            DESCRIPTOR,
            &["class:orders.Request", "class:orders.Response"],
            6000,
        );
        source_work.request.max_bytes = 2048;
        seal_work_identity(&mut source_work);
        let temp = tempfile::tempdir().unwrap();
        Repository::init(temp.path(), "API contract size admission").unwrap();
        let repo = Repository::open(temp.path()).unwrap();
        let mut selection = Selection::default();
        let mut source_omitted = false;
        for _ in 0..32 {
            let page = read_loaded(&repo, &source_work, selection.clone()).unwrap();
            source_omitted |= page["omitted"].as_array().is_some_and(|items| {
                items
                    .iter()
                    .any(|item| item["kind"] == "SOURCE" && item["id"] == "source-dto-2")
            });
            let Some(cursor) = page["nextCursor"].as_str() else {
                break;
            };
            selection.cursor = Some(cursor.into());
        }
        assert!(source_omitted);

        let source_reference = source_work
            .handles
            .iter()
            .find(|(_, handle)| handle.kind == "SOURCE" && handle.id == "source-dto-2")
            .map(|(reference, _)| reference.clone())
            .unwrap();
        let mut cursor = None;
        let mut received = String::new();
        loop {
            let part = super::super::work_parts::read_part_loaded(
                &repo,
                &source_work,
                SourcePartRequest {
                    schema: super::super::work_parts::REQUEST_SCHEMA.into(),
                    reference: source_reference.clone(),
                    cursor,
                },
            )
            .unwrap();
            received.push_str(part["text"].as_str().unwrap());
            cursor = part["nextCursor"].as_str().map(str::to_owned);
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(
            received,
            source_work.checked.services["orders"].sources["source-dto-2"].text
        );
        let source_state = read_state(&repo, &source_work.id).unwrap();
        assert!(initial_context_complete_with_parts(&source_work, &source_state).unwrap());

        let mut declaration_work = prepared_api_work(
            DESCRIPTOR,
            &["class:orders.Request", "class:orders.Response"],
            32,
        );
        declaration_work.request.max_bytes = 2048;
        let dependency = declaration_work
            .checked
            .dependencies
            .get_mut("dto-2")
            .unwrap();
        dependency.normalized["capturedPayload"] = json!("x".repeat(5000));
        dependency.digest = digest(&dependency.normalized).unwrap();
        let updated = dependency.clone();
        declaration_work
            .checked
            .services
            .get_mut("orders")
            .unwrap()
            .observations
            .insert("dto-2".into(), updated.clone());
        declaration_work
            .influence
            .insert("dto-2".into(), updated.digest.clone());
        let full_rows = rows(&declaration_work, &Selection::default()).unwrap();
        assert_eq!(
            full_rows
                .iter()
                .find(|row| row["kind"] == "DEPENDENCY" && row["id"] == "dto-2")
                .unwrap()["record"]["normalized"]["capturedPayload"]
                .as_str()
                .unwrap()
                .len(),
            5000
        );
        seal_work_identity(&mut declaration_work);
        let mut selection = Selection::default();
        let mut declaration_omitted = false;
        for _ in 0..32 {
            let page = read_loaded(&repo, &declaration_work, selection.clone()).unwrap();
            declaration_omitted |= page["omitted"].as_array().is_some_and(|items| {
                items
                    .iter()
                    .any(|item| item["kind"] == "DEPENDENCY" && item["id"] == "dto-2")
            });
            let Some(cursor) = page["nextCursor"].as_str() else {
                break;
            };
            selection.cursor = Some(cursor.into());
        }
        assert!(declaration_omitted);
        let declaration_state = read_state(&repo, &declaration_work.id).unwrap();
        assert!(
            !initial_context_complete_with_parts(&declaration_work, &declaration_state).unwrap()
        );
    }
}
