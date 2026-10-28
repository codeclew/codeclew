//! Human rendering and exact private lookup share an existing immutable Work.
//! The public view deliberately cannot serialize Source/Observation/Bindings.
mod human;

use super::{
    bytes, cache, digest, history, invalid, io_error,
    model::*,
    store::{self, Repository},
    work,
};
use crate::{canonical::hash_bytes, error::ClewError};
use clap::Subcommand;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
};

const SCHEMA: &str = "codeclew-reader-export/1.0";
const MAX_DOCUMENT: u64 = 4 * 1024 * 1024;
const MAX_PRIVATE: u64 = 16 * 1024 * 1024;

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Produce human-only HTML; evidence-bearing pages/render commands remain available.
    Export {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        work: String,
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    /// Read one exact exported section. No source acquisition or latest fallback.
    Query {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        reference: String,
        #[arg(long)]
        file: PathBuf,
        #[arg(long)]
        cursor: Option<String>,
        #[arg(long, default_value_t = 20)]
        max_items: usize,
        #[arg(long, default_value_t = 12288)]
        max_bytes: usize,
        /// Compare declarations with this separately supplied saved Check, never replace original evidence.
        #[arg(long)]
        compare_snapshot: Option<String>,
    },
    /// Recover exact original Markdown/map for caller review, without changing it.
    RepairBase {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        reference: String,
        #[arg(long)]
        file: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Request {
    schema: String,
    title: String,
    selection: Selection,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE", deny_unknown_fields)]
enum Selection {
    Native {
        bundle: String,
        operations: Vec<String>,
        /// Legacy boundary arrays mix business constraints and analysis caveats.
        /// Require exact caller classification; never infer the audience from words.
        #[serde(
            default,
            rename = "boundaryRoles",
            skip_serializing_if = "Vec::is_empty"
        )]
        boundary_roles: Vec<BoundaryRole>,
    },
    Note {
        id: String,
        sections: Vec<NoteSection>,
        /// Existing authored-sources map. Its native SOURCE identities are validated below.
        #[serde(default)]
        map: Option<PathBuf>,
        /// Optional explicit public Markdown block intervals. Defaults to the
        /// complete original prose; excludes declared provenance blocks only.
        #[serde(default, rename = "publicRanges")]
        public_ranges: Option<Vec<PublicRange>>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "field",
    rename_all = "SCREAMING_SNAKE_CASE",
    deny_unknown_fields
)]
enum BoundaryTarget {
    Operation { index: usize },
    Contract { id: String, index: usize },
    Visual { id: String, index: usize },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum BoundaryAudience {
    ReaderCondition,
    AnalysisCaveat,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BoundaryRole {
    operation: String,
    target: BoundaryTarget,
    text_digest: String,
    audience: BoundaryAudience,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NativeBoundary {
    target: BoundaryTarget,
    text_digest: String,
    text: String,
    audience: BoundaryAudience,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PublicRange {
    start_byte: usize,
    end_byte: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NoteSection {
    id: String,
    title: String,
    start_byte: usize,
    end_byte: usize,
    /// Typed registered binding slots; a section never guesses from its heading.
    #[serde(default)]
    bindings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SourceHandle {
    work: String,
    reference: String,
    record_digest: String,
    work_manifest_digest: String,
    snapshot: String,
    source: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Section {
    token: String,
    id: String,
    content_digest: String,
    origin: Value,
    sources: Vec<SourceHandle>,
    /// Human prose is retained privately too, without trusting a later edited export.
    text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    native_boundaries: Vec<NativeBoundary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Record {
    schema: String,
    work: String,
    namespace: String,
    snapshot: String,
    subject: String,
    work_manifest_digest: String,
    document_digest: String,
    file_digest: String,
    sections: Vec<Section>,
    #[serde(default)]
    markdown: Option<String>,
    #[serde(default)]
    source_map: Option<Value>,
    publication: Option<Value>,
    #[serde(default)]
    render_request: Option<Request>,
}

pub fn run(command: Command) -> Result<Value, ClewError> {
    match command {
        Command::Export {
            root,
            work,
            input,
            output,
        } => {
            let repo = Repository::open(&root)?;
            export(&repo, &work, store::read(&input, MAX_PRIVATE)?, &output)
        }
        Command::Query {
            root,
            reference,
            file,
            cursor,
            max_items,
            max_bytes,
            compare_snapshot,
        } => {
            let repo = Repository::open(&root)?;
            query(
                &repo,
                &reference,
                &file,
                cursor.as_deref(),
                max_items,
                max_bytes,
                compare_snapshot.as_deref(),
            )
        }
        Command::RepairBase {
            root,
            reference,
            file,
            output,
        } => {
            let repo = Repository::open(&root)?;
            repair_base(&repo, &reference, &file, &output)
        }
    }
}

fn read_file(path: &Path, limit: u64) -> Result<Vec<u8>, ClewError> {
    let meta = fs::symlink_metadata(path).map_err(io_error)?;
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > limit {
        return Err(invalid("reader input must be a bounded regular file"));
    }
    let data = fs::read(path).map_err(io_error)?;
    if data.len() as u64 > limit {
        return Err(invalid("reader input grew beyond its bound"));
    }
    Ok(data)
}

fn output_file(directory: &Path, name: &str, data: &[u8]) -> Result<(), ClewError> {
    if directory.exists() {
        return Err(invalid("reader output must be a new directory"));
    }
    fs::create_dir_all(directory).map_err(io_error)?;
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(directory.join(name))
        .map_err(io_error)?
        .write_all(data)
        .map_err(io_error)
}

fn handle(
    repo: &Repository,
    work: &work::Work,
    id: &str,
    expected: Option<&Source>,
) -> Result<SourceHandle, ClewError> {
    let sources = work.checked.sources();
    let source = sources
        .get(id)
        .ok_or_else(|| invalid("reader source is unavailable in its original Work"))?;
    if expected.is_some_and(|expected| expected != source) {
        return Err(invalid(
            "reader source differs from the original publication; use its original Work",
        ));
    }
    let refs = work
        .handles
        .iter()
        .filter(|(_, h)| h.kind == "SOURCE" && h.id == id)
        .collect::<Vec<_>>();
    if refs.len() != 1 {
        return Err(invalid("reader source handle is absent or ambiguous"));
    }
    Ok(SourceHandle {
        work: work.id.clone(),
        reference: refs[0].0.clone(),
        record_digest: digest(source)?,
        work_manifest_digest: work::retained_manifest_identity(repo, &work.id)?,
        snapshot: work
            .snapshot
            .clone()
            .ok_or_else(|| invalid("reader source Work has no original snapshot"))?,
        source: json!({"id":source.id,"service":source.service,"revision":source.revision,"file":source.file,"startLine":source.start_line,"endLine":source.end_line,"textDigest":source.text_digest,"authority":source.authority,"provider":work.checked.services.get(&source.service).map(|s|s.extractor.clone()),"coverage":work.checked.services.get(&source.service).map(|s|s.coverage.clone()),"profile":work.checked.source_inputs.as_ref().and_then(|i|i.inputs.services.get(&source.service)).map(|s|s.profile.clone()),"compilations":work.checked.source_inputs.as_ref().and_then(|i|i.inputs.services.get(&source.service)).map(|s|s.compilations.clone()),"url":source.url}),
    })
}

fn source_ids(value: &Value, ids: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                if key == "sourceIds" {
                    ids.extend(
                        value
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(Value::as_str)
                            .map(str::to_owned),
                    );
                } else {
                    source_ids(value, ids);
                }
            }
        }
        Value::Array(rows) => {
            for row in rows {
                source_ids(row, ids);
            }
        }
        _ => (),
    }
}

fn original_source(
    binding: &super::bindings::Bindings,
    subject: &str,
    operation: &str,
    id: &str,
) -> Result<Source, ClewError> {
    let prefix = format!("{subject}/{operation}/");
    let mut versions = BTreeMap::new();
    for (key, fragment) in &binding.fragments {
        if !key.starts_with(&prefix) {
            continue;
        }
        let Some(expected) = fragment.sources.get(id) else {
            continue;
        };
        let source = fragment
            .evidence
            .as_ref()
            .and_then(|e| e.sources.get(id))
            .or_else(|| binding.retained_sources.get(id))
            .ok_or_else(|| invalid("original fragment Source version is unavailable"))?;
        if expected["textDigest"] != source.text_digest
            || expected["revision"] != source.revision
            || expected["file"] != source.file
        {
            return Err(invalid(
                "original reader fragment differs from shared Source; no current-version fallback",
            ));
        }
        versions.insert(digest(source)?, source.clone());
    }
    if versions.len() != 1 {
        return Err(invalid(
            "reader section Source version is unavailable or ambiguous; select a smaller original fragment",
        ));
    }
    Ok(versions.into_values().next().unwrap())
}

fn markdown_block_range(text: &str, start: usize, end: usize) -> bool {
    let Some(before) = text.get(..start) else {
        return false;
    };
    let Some(after) = text.get(end..) else {
        return false;
    };
    let left = before.is_empty()
        || before.split('\n').rev().take(2).count() == 2
            && before
                .split('\n')
                .rev()
                .take(2)
                .all(|line| line.trim().is_empty());
    let right =
        after.trim().is_empty() || after.starts_with("\n\n") || after.starts_with("\r\n\r\n");
    left && right && start < end
}

fn classify_boundaries(
    op: &Operation,
    roles: &[BoundaryRole],
) -> Result<Vec<NativeBoundary>, ClewError> {
    let mut values = Vec::new();
    for (index, text) in op.boundaries.iter().enumerate() {
        values.push((BoundaryTarget::Operation { index }, text));
    }
    for contract in &op.interface_contracts {
        for (index, text) in contract.boundaries.iter().enumerate() {
            values.push((
                BoundaryTarget::Contract {
                    id: contract.id.clone(),
                    index,
                },
                text,
            ));
        }
    }
    for visual in &op.visuals {
        for (index, text) in visual.limitations.iter().enumerate() {
            values.push((
                BoundaryTarget::Visual {
                    id: visual.id.clone(),
                    index,
                },
                text,
            ));
        }
    }
    let selected: Vec<_> = roles.iter().filter(|r| r.operation == op.id).collect();
    if selected.len() != values.len() {
        return Err(invalid(
            "native boundaryRoles must classify every operation/contract/visual boundary exactly once",
        ));
    }
    values
        .into_iter()
        .map(|(target, text)| {
            let matches: Vec<_> = selected.iter().filter(|r| r.target == target).collect();
            if matches.len() != 1 || matches[0].text_digest != hash_bytes(text.as_bytes()) {
                return Err(invalid(
                    "native boundaryRoles target is duplicate, missing or has stale textDigest",
                ));
            }
            Ok(NativeBoundary {
                target,
                text_digest: matches[0].text_digest.clone(),
                text: text.clone(),
                audience: matches[0].audience.clone(),
            })
        })
        .collect()
}

fn export(
    repo: &Repository,
    id: &str,
    request: Request,
    output: &Path,
) -> Result<Value, ClewError> {
    if request.schema != "codeclew-reader-export-request/1.0"
        || request.title.trim().is_empty()
        || request.title.len() > 512
    {
        return Err(invalid("invalid human reader export request"));
    }
    let work = work::load(repo, id)?;
    let mut view = human::Document::new(request.title.clone());
    let mut record = Record {
        schema: SCHEMA.into(),
        work: id.into(),
        namespace: super::notes::source_bindings::namespace(repo)?,
        snapshot: work
            .snapshot
            .clone()
            .ok_or_else(|| invalid("reader export needs an immutable Work snapshot"))?,
        subject: work.subject.clone(),
        work_manifest_digest: work::retained_manifest_identity(repo, id)?,
        document_digest: String::new(),
        file_digest: String::new(),
        sections: vec![],
        markdown: None,
        source_map: None,
        publication: None,
        render_request: Some(request.clone()),
    };
    match &request.selection {
        Selection::Native {
            bundle,
            operations,
            boundary_roles,
        } => {
            if operations.is_empty()
                || operations.len() > 64
                || operations.iter().collect::<BTreeSet<_>>().len() != operations.len()
            {
                return Err(invalid(
                    "select 1..64 unique native operation/section identities",
                ));
            }
            if boundary_roles
                .iter()
                .any(|r| !operations.contains(&r.operation))
            {
                return Err(invalid(
                    "native boundaryRoles must belong to the explicitly selected operations",
                ));
            }
            let mut budget = history::FrozenInputBudget {
                remaining_bytes: 64 * 1024 * 1024,
            };
            let (publication, raw) = history::read_frozen_bindings(repo, bundle, &mut budget)?;
            let (_, binding) = history::validate_frozen_bindings(
                repo,
                publication.clone(),
                &raw,
                &mut BTreeMap::new(),
            )?;
            let narrative = binding
                .narratives
                .get(&work.subject)
                .ok_or_else(|| invalid("selected publication has no exact Work subject"))?;
            record.publication = Some(
                json!({"bundle":bundle,"publicationDigest":digest(&publication)?,"bindingsDigest":hash_bytes(&raw)}),
            );
            for selected in operations {
                let op = narrative
                    .operations
                    .iter()
                    .find(|op| &op.id == selected)
                    .ok_or_else(|| {
                        invalid(
                            "unknown selected native operation; heading inference is not supported",
                        )
                    })?;
                let raw = serde_json::to_value(op).map_err(io_error)?;
                let mut ids = BTreeSet::new();
                source_ids(&raw, &mut ids);
                // Original fragment Source versions must equal this admitted Work.
                // A same-named current source is never silently substituted.
                let sources = ids
                    .iter()
                    .map(|id| {
                        let expected = original_source(&binding, &work.subject, &op.id, id)?;
                        handle(repo, &work, id, Some(&expected))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let native_boundaries = classify_boundaries(op, boundary_roles)?;
                let conditions: Vec<_> = native_boundaries
                    .iter()
                    .filter(|b| b.audience == BoundaryAudience::ReaderCondition)
                    .map(|b| b.text.clone())
                    .collect();
                let section = human::operation(op, &conditions);
                record.sections.push(Section { token:String::new(), id:selected.clone(), content_digest:digest(op)?, origin:json!({"kind":"NATIVE","bundle":bundle,"subject":work.subject,"operation":selected}), sources, text:op.summary.text.clone(), native_boundaries });
                view.sections.push(section);
            }
        }
        Selection::Note {
            id: note_id,
            sections,
            map,
            public_ranges,
        } => {
            if sections.is_empty()
                || sections.len() > 128
                || sections
                    .iter()
                    .map(|s| &s.id)
                    .collect::<BTreeSet<_>>()
                    .len()
                    != sections.len()
            {
                return Err(invalid("select 1..128 unique explicit note sections"));
            }
            let note = work
                .checked
                .dependencies
                .get(&format!("note:{note_id}"))
                .filter(|n| n.kind == "NOTE_ASSOCIATION")
                .ok_or_else(|| invalid("registered note is absent from the original Work"))?;
            if work.subject != format!("service:{}", note.service)
                || note.normalized["association"]["id"] != *note_id
            {
                return Err(invalid(
                    "registered note belongs to a different admitted subject",
                ));
            }
            let original = note.normalized["original"]["text"]
                .as_str()
                .ok_or_else(|| invalid("registered note original is unavailable"))?;
            record.markdown = Some(original.into());
            if let Some(path) = map {
                let source_map: Value = store::read(path, MAX_PRIVATE)?;
                validate_map(repo, original, &source_map)?;
                record.source_map = Some(source_map);
            }
            let default_ranges = vec![PublicRange {
                start_byte: 0,
                end_byte: original.len(),
            }];
            let ranges = public_ranges.as_ref().unwrap_or(&default_ranges);
            if ranges.is_empty()
                || ranges.len() > 256
                || ranges.iter().any(|r| {
                    r.start_byte >= r.end_byte || original.get(r.start_byte..r.end_byte).is_none()
                })
                || ranges.windows(2).any(|r| r[0].end_byte > r[1].start_byte)
            {
                return Err(invalid(
                    "publicRanges require ordered, disjoint exact UTF-8 Markdown block intervals",
                ));
            }
            let mut ordered = sections.iter().collect::<Vec<_>>();
            ordered.sort_by_key(|s| s.start_byte);
            if ordered.windows(2).any(|s| s[0].end_byte > s[1].start_byte) {
                return Err(invalid("exported note sections must not overlap"));
            }
            let public_text = |start: usize, end: usize| -> String {
                ranges
                    .iter()
                    .filter_map(|r| {
                        let a = start.max(r.start_byte);
                        let b = end.min(r.end_byte);
                        (a < b).then(|| original[a..b].to_owned())
                    })
                    .collect::<Vec<_>>()
                    .join("\n\n")
            };
            let visible = public_text(0, original.len());
            view.original_heading = super::notes::protected_pages::body(&visible)
                .trim_start()
                .starts_with("# ");
            let mut previous = 0;
            let bindings = note.normalized["bindings"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            for selected in ordered {
                if !ranges
                    .iter()
                    .any(|r| r.start_byte <= selected.start_byte && selected.end_byte <= r.end_byte)
                {
                    return Err(invalid(
                        "selected note claim is outside declared publicRanges",
                    ));
                }
                if !markdown_block_range(original, selected.start_byte, selected.end_byte) {
                    return Err(invalid(
                        "human note export requires complete Markdown blocks; partial structural ranges are unsupported",
                    ));
                }
                if !store::valid_id(&selected.id) || selected.title.len() > 512 {
                    return Err(invalid("unsafe reader section identity/title"));
                }
                let text = original
                    .get(selected.start_byte..selected.end_byte)
                    .filter(|t| !t.is_empty())
                    .ok_or_else(|| {
                        invalid(
                            "note section range is empty, invalid UTF-8, or outside exact original",
                        )
                    })?;
                if record.source_map.as_ref().is_some_and(|m| {
                    !m["claims"].as_array().is_some_and(|claims| {
                        claims.iter().any(|c| {
                            c["id"] == selected.id
                                && c["text"] == text
                                && c["location"]["startByte"] == selected.start_byte
                                && c["location"]["endByte"] == selected.end_byte
                        })
                    })
                }) {
                    return Err(invalid(
                        "note section differs from the exact authored-sources claim map",
                    ));
                }
                let mut sources = vec![];
                for slot in &selected.bindings {
                    let b = bindings.iter().find(|b| b["id"] == *slot).ok_or_else(|| {
                        invalid("section binding slot is not registered on this note")
                    })?;
                    let source: Source =
                        serde_json::from_value(b["selectedSource"].clone()).map_err(io_error)?;
                    sources.push(handle(repo, &work, &source.id, Some(&source))?);
                }
                if let Some(map) = &record.source_map {
                    let claim = map["claims"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|c| c["id"] == selected.id)
                        .unwrap();
                    for source in claim["sources"].as_array().into_iter().flatten() {
                        sources.push(map_handle(repo, source)?);
                    }
                }
                if sources.is_empty() {
                    return Err(invalid(
                        "reader note section needs exact registered bindings or a validated native-source map",
                    ));
                }
                record.sections.push(Section { token:String::new(),id:selected.id.clone(),content_digest:hash_bytes(text.as_bytes()),origin:json!({"kind":"NOTE","note":note_id,"associationDigest":note.digest,"markdownDigest":hash_bytes(original.as_bytes()),"mapDigest":record.source_map.as_ref().map(|m|m["mapDigest"].clone()),"startByte":selected.start_byte,"endByte":selected.end_byte}),sources,text:text.into(),native_boundaries:vec![] });
                let mut section = human::Section::markdown(selected.title.clone(), text.into());
                human::surround(
                    &mut section,
                    public_text(previous, selected.start_byte),
                    String::new(),
                );
                view.sections.push(section);
                previous = selected.end_byte;
            }
            if let Some(last) = view.sections.last_mut() {
                human::surround_after(last, public_text(previous, original.len()));
            }
        }
    }
    address_view(&mut record, &mut view)?;
    let html = human::render(&view)?;
    if html.len() as u64 > MAX_DOCUMENT {
        return Err(invalid("human HTML exceeds 4 MiB; select fewer sections"));
    }
    record.file_digest = hash_bytes(html.as_bytes());
    let raw = bytes(&record)?;
    if raw.len() as u64 > MAX_PRIVATE {
        return Err(invalid("reader receipt exceeds 16 MiB"));
    }
    if output.exists() {
        return Err(invalid("reader output must be a new directory"));
    }
    // Existing lifecycle: durable object before its existing Work retention root.
    let _lock = repo.lock()?;
    let mut state = work::read_state(repo, id)?;
    if state.work != id || state.reader_exports.len() + record.sections.len() > 2048 {
        return Err(invalid(
            "reader ledger identity or export count exceeds its bound",
        ));
    }
    let reference = cache::put(repo, SCHEMA, &raw)?;
    for section in &record.sections {
        if state
            .reader_exports
            .get(&section.token)
            .is_some_and(|old| old != &reference)
        {
            return Err(invalid(
                "reader export identity already has different immutable bytes",
            ));
        }
        state
            .reader_exports
            .insert(section.token.clone(), reference.clone());
    }
    let ledger = bytes(&state)?;
    if ledger.len() as u64 > MAX_PRIVATE {
        return Err(invalid("reader Work ledger exceeds 16 MiB"));
    }
    repo.atomic(&format!("{}/reads.json", work::directory(id)?), &ledger)?;
    output_file(output, "index.html", html.as_bytes())?;
    Ok(
        json!({"schema":SCHEMA,"status":"EXPORTED_HUMAN_ONLY","file":output.join("index.html"),"fileDigest":record.file_digest,"snapshot":record.snapshot,"work":id,"sections":record.sections.iter().map(|s|json!({"id":s.id,"reference":format!("clew-reader-v1:{id}:{}",s.token)})).collect::<Vec<_>>(),"machineMetadata":"RETAINED_IN_EXISTING_WORK_LEDGER","meaning":"UNASSESSED; retained evidence is not runtime verification"}),
    )
}

fn address_view(record: &mut Record, view: &mut human::Document) -> Result<(), ClewError> {
    // Reference-free HTML binds the actual renderer without a self-hash cycle.
    // A renderer upgrade can coexist with old retained exports of the same prose.
    for section in &mut record.sections {
        section.token.clear();
    }
    for section in &mut view.sections {
        section.reference.clear();
    }
    let render_digest = hash_bytes(human::render(view)?.as_bytes());
    record.document_digest = digest(&(
        &record.render_request,
        &record.sections,
        &record.publication,
        &record.markdown,
        &record.source_map,
        &render_digest,
    ))?;
    for (index, section) in record.sections.iter_mut().enumerate() {
        section.token = digest(&(
            &record.namespace,
            &record.work,
            &record.document_digest,
            index,
            &section.id,
            &section.content_digest,
        ))?[7..]
            .into();
        view.sections[index].reference =
            format!("clew-reader-v1:{}:{}", record.work, section.token);
    }
    Ok(())
}

fn map_handle(repo: &Repository, value: &Value) -> Result<SourceHandle, ClewError> {
    let id = value["work"]
        .as_str()
        .ok_or_else(|| invalid("source map lacks exact native Work identity"))?;
    let saved = work::load(repo, id)?;
    let source_id = value["id"]
        .as_str()
        .ok_or_else(|| invalid("source map lacks exact Source identity"))?;
    let sources = saved.checked.sources();
    let source = sources
        .get(source_id)
        .ok_or_else(|| invalid("source map original Source is unavailable"))?;
    if saved.snapshot.as_deref() != value["snapshot"].as_str()
        || value["revision"] != source.revision
        || value["textDigest"] != source.text_digest
        || value["file"] != source.file
        || value["startLine"] != source.start_line
        || value["endLine"] != source.end_line
    {
        return Err(invalid(
            "source map differs from original native Source binding",
        ));
    }
    let resolved = handle(repo, &saved, source_id, None)?;
    if value["sourceReference"] != resolved.reference {
        return Err(invalid(
            "source map Work reference does not select the retained Source",
        ));
    }
    Ok(resolved)
}

fn validate_map(repo: &Repository, markdown: &str, map: &Value) -> Result<(), ClewError> {
    let mut body = map.clone();
    body.as_object_mut()
        .ok_or_else(|| invalid("invalid source map"))?
        .remove("mapDigest");
    if map["schema"] != "codeclew-authored-sources/1.0"
        || map["markdownDigest"] != hash_bytes(markdown.as_bytes())
        || map["mapDigest"] != digest(&body)?
    {
        return Err(invalid("source map or exact Markdown digest differs"));
    }
    let claims = map["claims"]
        .as_array()
        .filter(|c| !c.is_empty() && c.len() <= 1024)
        .ok_or_else(|| invalid("source map claims exceed bounded supported schema"))?;
    let mut ids = BTreeSet::new();
    for claim in claims {
        let id = claim["id"]
            .as_str()
            .ok_or_else(|| invalid("source map claim has no identity"))?;
        if !ids.insert(id) {
            return Err(invalid("ambiguous source map claim"));
        }
        let a = claim["location"]["startByte"]
            .as_u64()
            .ok_or_else(|| invalid("source map claim has no byte range"))? as usize;
        let b = claim["location"]["endByte"]
            .as_u64()
            .ok_or_else(|| invalid("source map claim has no byte range"))? as usize;
        if markdown.get(a..b) != claim["text"].as_str() {
            return Err(invalid(
                "source map claim does not address exact Markdown bytes",
            ));
        }
        for source in claim["sources"].as_array().into_iter().flatten() {
            map_handle(repo, source)?;
        }
    }
    Ok(())
}

fn resolve(repo: &Repository, reference: &str, file: &Path) -> Result<(Record, usize), ClewError> {
    let tokens = reference.split(':').collect::<Vec<_>>();
    if tokens.len() != 3
        || tokens[0] != "clew-reader-v1"
        || tokens[2].len() != 64
        || !tokens[2]
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err(invalid("invalid opaque reader reference"));
    }
    work::directory(tokens[1])?;
    let state = work::read_state(repo, tokens[1])?;
    if state.work != tokens[1] {
        return Err(invalid("reader registry Work is unavailable"));
    }
    let object = state.reader_exports.get(tokens[2]).ok_or_else(|| {
        invalid("reader reference is unavailable in this originating registry; no latest fallback")
    })?;
    if object.schema != SCHEMA {
        return Err(invalid("reader receipt schema mismatch"));
    }
    let raw = cache::get(repo, object, MAX_PRIVATE)?.ok_or_else(|| {
        invalid("reader history was collected or is unavailable; no latest fallback")
    })?;
    let record: Record = serde_json::from_slice(&raw).map_err(io_error)?;
    if record.schema != SCHEMA
        || record.work != tokens[1]
        || record.file_digest != hash_bytes(&read_file(file, MAX_DOCUMENT)?)
    {
        return Err(invalid(
            "reader document was modified or does not match this exact export",
        ));
    }
    let indexes = record
        .sections
        .iter()
        .enumerate()
        .filter(|(_, s)| s.token == tokens[2])
        .map(|(i, _)| i)
        .collect::<Vec<_>>();
    if indexes.len() != 1 {
        return Err(invalid("reader section is missing or ambiguous"));
    }
    // Loading this exact Work validates retention, never selects another Work.
    if work::retained_manifest_identity(repo, &record.work)? != record.work_manifest_digest {
        return Err(invalid("reader original Work manifest changed"));
    }
    Ok((record, indexes[0]))
}

fn query(
    repo: &Repository,
    reference: &str,
    file: &Path,
    cursor: Option<&str>,
    max_items: usize,
    max_bytes: usize,
    compare_snapshot: Option<&str>,
) -> Result<Value, ClewError> {
    if !(1..=100).contains(&max_items) || !(2048..=49152).contains(&max_bytes) {
        return Err(invalid(
            "reader query requires 1..100 items and 2048..49152 bytes",
        ));
    }
    let (record, index) = resolve(repo, reference, file)?;
    let section = &record.sections[index];
    let compared = compare_snapshot
        .map(|snapshot| super::check::Check::load_snapshot(repo, snapshot))
        .transpose()?;
    let current_sources = compared.as_ref().map(|c| c.sources());
    let mut changed = 0usize;
    let mut manifests = BTreeMap::new();
    let render_policy=record.render_request.as_ref().map(|request|match &request.selection {
        Selection::Native{bundle,..}=>json!({"kind":"NATIVE","bundle":bundle,"operation":section.id}),
        Selection::Note{public_ranges,..}=>json!({"kind":"NOTE","publicRanges":public_ranges,"selectedSection":section.id,"scope":"Selected section metadata only; complete original request retained privately"}),
    });
    let mut items = vec![
        json!({"kind":"SECTION","id":section.id,"origin":section.origin,"contentDigest":section.content_digest,"snapshot":record.snapshot,"subject":record.subject,"documentDigest":record.document_digest,"fileDigest":record.file_digest,"publication":record.publication,"renderPolicy":render_policy,"repairSupported":false,"repairBaseAvailable":record.source_map.is_some(),"repairLimit":"Original Markdown/map export only; no automated paragraph repair adapter"}),
    ];
    // Original untyped boundaries and the exact caller projection stay private.
    // One item per boundary uses the same byte/item/cursor limits as Source rows.
    for boundary in &section.native_boundaries {
        items.push(json!({"kind":"NATIVE_BOUNDARY","target":boundary.target,"textDigest":boundary.text_digest,"text":boundary.text,"audience":boundary.audience,"classificationAuthority":"EXPLICIT_EXPORT_CALLER_NOT_SEMANTIC_VERIFICATION"}));
    }
    for handle in &section.sources {
        if !manifests.contains_key(&handle.work) {
            manifests.insert(
                handle.work.clone(),
                work::retained_manifest_identity(repo, &handle.work)?,
            );
        }
        if manifests[&handle.work] != handle.work_manifest_digest {
            return Err(invalid("reader original Source Work manifest changed"));
        }
        let state = if let Some(current) = &current_sources {
            match handle.source["id"].as_str().and_then(|id| current.get(id)) {
                None => "MISSING_FROM_COMPARED_CHECK",
                Some(source)
                    if source.revision == handle.source["revision"]
                        && source.text_digest == handle.source["textDigest"]
                        && source.file == handle.source["file"]
                        && source.start_line == handle.source["startLine"]
                        && source.end_line == handle.source["endLine"] =>
                {
                    "SAME_RETAINED_SOURCE_VERSION"
                }
                Some(source) if source.text_digest == handle.source["textDigest"] => {
                    "VERSION_OR_LOCATION_CHANGED_SAME_TEXT"
                }
                Some(_) => "SOURCE_TEXT_CHANGED",
            }
        } else {
            "NOT_COMPARED"
        };
        if !matches!(state, "NOT_COMPARED" | "SAME_RETAINED_SOURCE_VERSION") {
            changed += 1;
        }
        items.push(json!({"kind":"SOURCE_BINDING","work":handle.work,"reference":handle.reference,"snapshot":handle.snapshot,"sourceId":handle.source["id"],"revision":handle.source["revision"],"file":handle.source["file"],"startLine":handle.source["startLine"],"endLine":handle.source["endLine"],"textDigest":handle.source["textDigest"],"recordDigest":handle.record_digest,"sourceAuthority":handle.source["authority"],"provider":handle.source["provider"],"coverage":handle.source["coverage"],"profile":handle.source["profile"],"compilations":handle.source["compilations"],"workContextRead":{"command":"docs work read","input":{}},"comparison":state,"readPart":{"command":"docs work read-part","input":{"schema":"codeclew-documentation-source-part-request/1.0","reference":handle.reference,"cursor":null}}}));
    }
    let note_changed = compared.as_ref().and_then(|c| {
        section.origin["note"].as_str().map(|id| {
            c.dependencies.get(&format!("note:{id}")).is_none_or(|n| {
                n.normalized["original"]["text"]
                    .as_str()
                    .is_none_or(|t| hash_bytes(t.as_bytes()) != section.origin["markdownDigest"])
            })
        })
    });
    let comparison = if let Some(snapshot) = compare_snapshot {
        json!({"status":if changed>0 || note_changed==Some(true){"DIFFERENT_RETAINED_BINDINGS"}else{"MATCHING_RETAINED_BINDINGS"},"snapshot":snapshot,"originalSnapshot":record.snapshot,"changedSources":changed,"noteContentChanged":note_changed,"authority":"SEPARATE_CHECK_COMPARISON_NOT_SOURCE_REVALIDATION"})
    } else {
        json!({"status":"NOT_COMPARED","authority":"RETAINED_ORIGINAL_NOT_REVERIFIED"})
    };
    let binding = digest(&(
        reference,
        &record.file_digest,
        &section.content_digest,
        max_items,
        max_bytes,
        compare_snapshot,
    ))?;
    let start = match cursor {
        None => 0,
        Some(cursor) => {
            let (prefix, offset) = cursor
                .rsplit_once(':')
                .ok_or_else(|| invalid("invalid reader cursor"))?;
            if prefix != binding {
                return Err(invalid(
                    "reader cursor belongs to another version/selection/budget",
                ));
            }
            offset
                .parse::<usize>()
                .map_err(|_| invalid("invalid reader cursor offset"))?
        }
    };
    if start > items.len() {
        return Err(invalid("reader cursor is outside selection"));
    }
    let mut response = json!({"schema":"codeclew-reader-query/1.0","reference":reference,"authority":"EXACT_RETAINED_EXPORT_NOT_REVERIFIED","freshness":comparison,"items":[],"total":items.len(),"omitted":[],"nextCursor":null});
    let mut end = start;
    let mut selected = vec![];
    for item in items.iter().skip(start).take(max_items) {
        selected.push(item.clone());
        response["items"] = json!(selected);
        if bytes(&response)?.len() + 256 > max_bytes {
            selected.pop();
            response["items"] = json!(selected);
            break;
        }
        end += 1;
    }
    if end == start && end < items.len() {
        return Err(invalid(
            "reader row exceeds query budget; increase max-bytes up to 49152",
        ));
    }
    if end < items.len() {
        response["nextCursor"] = json!(format!("{binding}:{end}"));
    }
    Ok(response)
}

fn repair_base(
    repo: &Repository,
    reference: &str,
    file: &Path,
    output: &Path,
) -> Result<Value, ClewError> {
    let (record, index) = resolve(repo, reference, file)?;
    let section = &record.sections[index];
    let markdown=record.markdown.as_ref().ok_or_else(||invalid("native section repair uses its exact native historical selector; generic Markdown repair is unsupported"))?;
    let map = record.source_map.as_ref().ok_or_else(|| {
        invalid("registered page has no exact guarded Markdown source map; repair unsupported")
    })?;
    validate_map(repo, markdown, map)?;
    output_file(output, "index.md", markdown.as_bytes())?;
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output.join("sources.json"))
        .map_err(io_error)?
        .write_all(&bytes(map)?)
        .map_err(io_error)?;
    Ok(
        json!({"status":"EXACT_READER_REPAIR_BASE_EXPORTED","reference":reference,"claim":section.id,"old":section.text,"baseMarkdownDigest":hash_bytes(markdown.as_bytes()),"baseMapDigest":map["mapDigest"],"base":output,"next":"Original Markdown and source map exported for caller review. No automated repair adapter or meaning approval is included.","unsupported":"List/table/code/link structural mutation; preserve existing guarded paragraph rules."}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, Repository, work::Work) {
        let temporary = tempfile::tempdir().unwrap();
        Repository::init(temporary.path(), "Owned reader fixture").unwrap();
        let repo = Repository::open(temporary.path()).unwrap();
        let mut saved = work::api_contract_tests::endpoint_context_fixture();
        work::api_contract_tests::persist_operation_fixture(&repo, &mut saved);
        (temporary, repo, saved)
    }

    fn receipt(repo: &Repository, saved: &work::Work, file: &Path) -> String {
        let source = saved.checked.sources().into_values().next().unwrap();
        let source = handle(repo, saved, &source.id, None).unwrap();
        let token = "a".repeat(64);
        let record = Record {
            schema: SCHEMA.into(),
            work: saved.id.clone(),
            namespace: super::super::notes::source_bindings::namespace(repo).unwrap(),
            snapshot: saved.snapshot.clone().unwrap(),
            subject: saved.subject.clone(),
            work_manifest_digest: work::retained_manifest_identity(repo, &saved.id).unwrap(),
            document_digest: hash_bytes(b"document"),
            file_digest: hash_bytes(&fs::read(file).unwrap()),
            sections: vec![Section {
                token: token.clone(),
                id: "shared-section-name".into(),
                content_digest: hash_bytes(b"paragraph"),
                origin: json!({"kind":"TEST_OWNED"}),
                sources: vec![source],
                text: "paragraph".into(),
                native_boundaries: vec![],
            }],
            markdown: None,
            source_map: None,
            publication: None,
            render_request: None,
        };
        let object = cache::put(repo, SCHEMA, &bytes(&record).unwrap()).unwrap();
        let mut state = work::read_state(repo, &saved.id).unwrap();
        state.reader_exports.insert(token.clone(), object);
        repo.atomic(
            &format!("{}/reads.json", work::directory(&saved.id).unwrap()),
            &bytes(&state).unwrap(),
        )
        .unwrap();
        format!("clew-reader-v1:{}:{token}", saved.id)
    }

    #[test]
    fn exact_reader_rejects_modified_file_foreign_registry_and_rebound_cursor() {
        let (tmp, repo, saved) = fixture();
        let file = tmp.path().join("export.html");
        fs::write(&file, b"<main>Original paragraph</main>").unwrap();
        let reference = receipt(&repo, &saved, &file);
        let first = query(&repo, &reference, &file, None, 1, 4096, None).unwrap();
        assert_eq!(first["items"].as_array().unwrap().len(), 1);
        assert_eq!(first["freshness"]["status"], "NOT_COMPARED");
        let cursor = first["nextCursor"].as_str().unwrap();
        let second = query(&repo, &reference, &file, Some(cursor), 1, 4096, None).unwrap();
        assert_eq!(second["items"][0]["kind"], "SOURCE_BINDING");
        assert!(second["nextCursor"].is_null());
        assert!(query(&repo, &reference, &file, Some(cursor), 2, 4096, None).is_err());
        let (_other, other, foreign) = fixture();
        assert_eq!(
            foreign.id, saved.id,
            "equal immutable Work inputs are intentionally shared; export namespace must still isolate registries"
        );
        assert!(resolve(&other, &reference, &file).is_err());
        fs::write(&file, b"<main>Edited paragraph</main>").unwrap();
        assert!(resolve(&repo, &reference, &file).is_err());
    }

    #[test]
    fn new_check_is_comparison_only_and_never_rebinds_original_source() {
        let (tmp, repo, saved) = fixture();
        let file = tmp.path().join("copy.html");
        fs::write(&file, b"<main>Exact old export</main>").unwrap();
        let reference = receipt(&repo, &saved, &file);
        let mut changed = saved.checked.clone();
        for service in changed.services.values_mut() {
            service.revision = "new-owned-revision".into();
            for source in service.sources.values_mut() {
                source.revision = "new-owned-revision".into();
            }
        }
        changed.refresh_digest().unwrap();
        let snapshot = changed.save_snapshot(&repo).unwrap();
        let response = query(&repo, &reference, &file, None, 20, 12288, Some(&snapshot)).unwrap();
        assert_eq!(
            response["freshness"]["status"],
            "DIFFERENT_RETAINED_BINDINGS"
        );
        assert_ne!(response["items"][1]["revision"], "new-owned-revision");
        assert_eq!(response["items"][1]["snapshot"], saved.snapshot.unwrap());
    }

    #[test]
    fn original_receipt_survives_prose_update_and_storage_prune() {
        let (tmp, repo, saved) = fixture();
        let file = tmp.path().join("export.html");
        fs::write(&file, b"<main>Old export</main>").unwrap();
        let reference = receipt(&repo, &saved, &file);
        repo.atomic(
            "notes/newer.md",
            b"New prose, old export still addresses old Work",
        )
        .unwrap();
        assert_eq!(
            resolve(&repo, &reference, &file).unwrap().0.snapshot,
            saved.snapshot.unwrap()
        );
        let state = work::read_state(&repo, &saved.id).unwrap();
        let object = state.reader_exports.values().next().unwrap();
        // Existing storage planner must treat the typed object in the existing
        // Work read ledger as a retention root, not collect it as unknown cache.
        let plan = super::super::storage::run(super::super::storage::Command::Plan {
            root: repo.root.clone(),
        })
        .unwrap();
        assert!(
            serde_json::to_string(&plan)
                .unwrap()
                .contains(&object.digest)
        );
        let root = repo.root.clone();
        let expected = plan["planDigest"].as_str().unwrap().to_owned();
        drop(repo);
        super::super::storage::run(super::super::storage::Command::Prune {
            root: root.clone(),
            expected_plan: expected,
            apply: true,
        })
        .unwrap();
        let repo = Repository::open(&root).unwrap();
        assert!(resolve(&repo, &reference, &file).is_ok());
    }

    #[test]
    fn registered_note_ranges_resolve_independently_and_keep_old_versions() {
        let (tmp, repo, mut saved) = fixture();
        let source = saved.checked.sources().into_values().next().unwrap();
        let markdown = "Preserve `cityId` and [the contract](https://example.invalid/contract).\n\nField | Meaning\n--- | ---\nid | Identifier\n";
        let split = markdown.find("\n\n").unwrap();
        let normalized = json!({"association":{"id":"manual","service":"orders","title":"Manual","path":"notes/manual.md"},"original":{"text":markdown,"digest":digest(&markdown).unwrap()},"bindings":[{"id":"binding","selectedSource":source}]});
        saved.checked.dependencies.insert(
            "note:manual".into(),
            Observation {
                id: "note:manual".into(),
                kind: "NOTE_ASSOCIATION".into(),
                service: "orders".into(),
                symbol: "Manual".into(),
                digest: digest(&normalized).unwrap(),
                normalized,
                source_ids: vec![],
            },
        );
        work::api_contract_tests::persist_operation_fixture(&repo, &mut saved);
        let request = Request {
            schema: "codeclew-reader-export-request/1.0".into(),
            title: "Manual".into(),
            selection: Selection::Note {
                id: "manual".into(),
                map: None,
                public_ranges: None,
                sections: vec![
                    NoteSection {
                        id: "intro".into(),
                        title: "Repeated heading".into(),
                        start_byte: 0,
                        end_byte: split,
                        bindings: vec!["binding".into()],
                    },
                    NoteSection {
                        id: "fields".into(),
                        title: "Repeated heading".into(),
                        start_byte: split + 2,
                        end_byte: markdown.len(),
                        bindings: vec!["binding".into()],
                    },
                ],
            },
        };
        let output = tmp.path().join("human");
        let result = export(&repo, &saved.id, request, &output).unwrap();
        let first = result["sections"][0]["reference"].as_str().unwrap();
        let second = result["sections"][1]["reference"].as_str().unwrap();
        assert_ne!(first, second);
        let file = output.join("index.html");
        let first_page = query(&repo, first, &file, None, 1, 4096, None).unwrap();
        assert!(
            query(
                &repo,
                second,
                &file,
                first_page["nextCursor"].as_str(),
                1,
                4096,
                None
            )
            .is_err()
        );
        assert_eq!(
            query(&repo, first, &file, None, 20, 12288, None).unwrap()["items"][0]["origin"]["startByte"],
            0
        );
        assert_eq!(
            query(&repo, second, &file, None, 20, 12288, None).unwrap()["items"][0]["origin"]["startByte"],
            split + 2
        );
        assert!(repair_base(&repo, second, &file, &tmp.path().join("unsupported")).is_err());
        let html = fs::read_to_string(&file).unwrap();
        assert!(html.contains("Field | Meaning") && html.contains("Preserve `cityId`"));
        assert!(
            !html.contains(&source.text_digest)
                && !html.contains("selectedSource")
                && !html.contains("association")
        );
    }

    #[test]
    fn native_section_exports_from_exact_frozen_publication() {
        let (tmp, repo, saved) = fixture();
        let source = saved
            .checked
            .sources()
            .into_values()
            .find(|s| s.id == "endpoint-source")
            .unwrap();
        let dependency = saved
            .checked
            .dependencies
            .values()
            .find(|d| d.source_ids.contains(&source.id))
            .unwrap();
        let mut narrative:Narrative=serde_json::from_value(json!({"schema":"codeclew-documentation-narrative/1.3","subject":"service:orders","contextDigest":saved.checked.context_digest,"operations":[{"id":"section-overview","title":"Owned service overview","summary":{"id":"purpose","text":"The service accepts a request and delegates processing.","sourceIds":[source.id],"dependencyIds":[dependency.id]},"participants":[],"events":[],"findings":[],"boundaries":["Static explanation; runtime unverified","Delivery status remains unknown until publisher acknowledgement."]}],"gaps":{}})).unwrap();
        narrative.gaps = super::super::notes::expected(&saved.checked, "orders")
            .into_iter()
            .filter(|id| id != "section-overview")
            .map(|id| (id, "Owned fixture does not author this operation".into()))
            .collect();
        super::super::render::validate(&narrative, &saved.checked).unwrap();
        let published = super::super::render::publish_from_snapshot(
            &repo,
            vec![narrative],
            false,
            BTreeMap::new(),
            saved.snapshot.as_deref().unwrap(),
        )
        .unwrap();
        let bundle = published["bundle"].as_str().unwrap();
        let request = Request {
            schema: "codeclew-reader-export-request/1.0".into(),
            title: "Owned native page".into(),
            selection: Selection::Native {
                bundle: bundle.into(),
                operations: vec!["section-overview".into()],
                boundary_roles: vec![
                    BoundaryRole {
                        operation: "section-overview".into(),
                        target: BoundaryTarget::Operation { index: 0 },
                        text_digest: hash_bytes(b"Static explanation; runtime unverified"),
                        audience: BoundaryAudience::AnalysisCaveat,
                    },
                    BoundaryRole {
                        operation: "section-overview".into(),
                        target: BoundaryTarget::Operation { index: 1 },
                        text_digest: hash_bytes(
                            b"Delivery status remains unknown until publisher acknowledgement.",
                        ),
                        audience: BoundaryAudience::ReaderCondition,
                    },
                ],
            },
        };
        let output = tmp.path().join("native-human");
        let mut invalid_request = request.clone();
        if let Selection::Native { boundary_roles, .. } = &mut invalid_request.selection {
            boundary_roles.clear();
        }
        assert!(export(&repo, &saved.id, invalid_request, &output).is_err());
        let mut stale_request = request.clone();
        if let Selection::Native { boundary_roles, .. } = &mut stale_request.selection {
            boundary_roles[0].text_digest = hash_bytes(b"changed");
        }
        assert!(export(&repo, &saved.id, stale_request, &output).is_err());
        let result = export(&repo, &saved.id, request, &output).unwrap();
        let reference = result["sections"][0]["reference"].as_str().unwrap();
        let html = fs::read_to_string(output.join("index.html")).unwrap();
        assert!(html.contains("Delivery status remains unknown until publisher acknowledgement."));
        assert!(!html.contains("Static explanation; runtime unverified"));
        assert!(!html.contains("What remains uncertain"));
        let value = query(
            &repo,
            reference,
            &output.join("index.html"),
            None,
            20,
            12288,
            None,
        )
        .unwrap();
        assert_eq!(value["items"][0]["origin"]["bundle"], bundle);
        assert_eq!(value["items"][0]["origin"]["operation"], "section-overview");
        assert!(
            value["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["kind"] == "NATIVE_BOUNDARY"
                    && row["audience"] == "ANALYSIS_CAVEAT"
                    && row["text"] == "Static explanation; runtime unverified")
        );
        let mut cursor = None;
        let mut paged = vec![];
        loop {
            let page = query(
                &repo,
                reference,
                &output.join("index.html"),
                cursor.as_deref(),
                1,
                2048,
                None,
            )
            .unwrap();
            assert_eq!(page["items"].as_array().unwrap().len(), 1);
            assert!(bytes(&page).unwrap().len() <= 2048);
            paged.extend(page["items"].as_array().unwrap().clone());
            cursor = page["nextCursor"].as_str().map(String::from);
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(paged, *value["items"].as_array().unwrap());
        assert_eq!(fs::read_dir(&output).unwrap().count(), 1);
        // This profile never replaces or mutates the legacy evidence bundle.
        assert!(
            repo.path(&format!("docs/generated/{bundle}/bindings.json"))
                .unwrap()
                .is_file()
        );
    }

    #[test]
    fn note_preserves_surrounding_content_and_explicit_public_range_policy() {
        let (tmp, repo, mut saved) = fixture();
        let source = saved.checked.sources().into_values().next().unwrap();
        let text = "# Manual\n\nKeep the introduction.\n\nAudit: sha256:OWNED_AUDIT_SENTINEL\n\nPreserve `cityId`.\n\nUnchanged closing explanation.\n";
        let audit = text.find("Audit:").unwrap();
        let selected = text.find("Preserve").unwrap();
        let end = selected + "Preserve `cityId`.".len();
        let normalized = json!({"association":{"id":"manual","service":"orders"},"original":{"text":text},"bindings":[{"id":"binding","selectedSource":source}]});
        saved.checked.dependencies.insert(
            "note:manual".into(),
            Observation {
                id: "note:manual".into(),
                kind: "NOTE_ASSOCIATION".into(),
                service: "orders".into(),
                symbol: "Manual".into(),
                digest: digest(&normalized).unwrap(),
                normalized,
                source_ids: vec![],
            },
        );
        work::api_contract_tests::persist_operation_fixture(&repo, &mut saved);
        let make = |ranges| Request {
            schema: "codeclew-reader-export-request/1.0".into(),
            title: "Manual".into(),
            selection: Selection::Note {
                id: "manual".into(),
                map: None,
                public_ranges: ranges,
                sections: vec![NoteSection {
                    id: "paragraph".into(),
                    title: "Paragraph".into(),
                    start_byte: selected,
                    end_byte: end,
                    bindings: vec!["binding".into()],
                }],
            },
        };
        let full = tmp.path().join("full");
        let result = export(&repo, &saved.id, make(None), &full).unwrap();
        let full_html = fs::read_to_string(full.join("index.html")).unwrap();
        assert!(
            full_html.contains("Keep the introduction.")
                && full_html.contains("Unchanged closing explanation.")
        );
        // Declared prose is never guessed away. The caller explicitly selects
        // public intervals when original notes also contain provenance prose.
        assert!(full_html.contains("OWNED_AUDIT_SENTINEL"));
        let selected_dir = tmp.path().join("selected");
        let hidden = Some(vec![
            PublicRange {
                start_byte: 0,
                end_byte: audit,
            },
            PublicRange {
                start_byte: selected,
                end_byte: text.len(),
            },
        ]);
        let projected = export(&repo, &saved.id, make(hidden), &selected_dir).unwrap();
        let html = fs::read_to_string(selected_dir.join("index.html")).unwrap();
        assert!(!html.contains("OWNED_AUDIT_SENTINEL"));
        assert!(
            html.contains("Keep the introduction.")
                && html.contains("Unchanged closing explanation.")
                && html.contains("Preserve `cityId`.")
        );
        assert_ne!(
            result["sections"][0]["reference"],
            projected["sections"][0]["reference"]
        );
        let reference = projected["sections"][0]["reference"].as_str().unwrap();
        let (receipt, _) = resolve(&repo, reference, &selected_dir.join("index.html")).unwrap();
        assert_eq!(receipt.markdown.as_deref(), Some(text));
        assert!(receipt.render_request.is_some());
    }

    #[test]
    fn public_view_keeps_prose_contracts_and_diagrams_without_native_payload() {
        let op:Operation=serde_json::from_value(json!({"id":"section-overview","title":"Save a city","summary":{"id":"summary","text":"Save `cityId` and publish an event.","sourceIds":["PRIVATE_SOURCE_SENTINEL"],"dependencyIds":["PRIVATE_BINDING_SENTINEL"]},"participants":[{"id":"one","label":"Dashboard","service":"dashboard"},{"id":"two","label":"Forecasts","service":"forecasts"}],"events":[{"id":"call","kind":"message","text":"Send the request; failures remain visible.","from":"one","to":"two","sourceIds":["PRIVATE_SOURCE_SENTINEL"],"dependencyIds":[]}],"explanation":[{"id":"details","text":"Errors keep the request pending.","eventIds":["call"],"sourceIds":[],"dependencyIds":[],"detail":true}],"interfaceContracts":[{"id":"request","title":"Request","kind":"http","rows":[{"id":"city","label":"cityId","value":"Required string","sourceIds":["PRIVATE_SOURCE_SENTINEL"],"dependencyIds":[]}],"boundaries":["Runtime compatibility unverified"]}],"findings":[],"boundaries":["Delivery is not guaranteed"]})).unwrap();
        let mut document = human::Document::new("Owned public view".into());
        let roles = vec![
            BoundaryRole {
                operation: op.id.clone(),
                target: BoundaryTarget::Operation { index: 0 },
                text_digest: hash_bytes(op.boundaries[0].as_bytes()),
                audience: BoundaryAudience::ReaderCondition,
            },
            BoundaryRole {
                operation: op.id.clone(),
                target: BoundaryTarget::Contract {
                    id: "request".into(),
                    index: 0,
                },
                text_digest: hash_bytes(op.interface_contracts[0].boundaries[0].as_bytes()),
                audience: BoundaryAudience::AnalysisCaveat,
            },
        ];
        let boundaries = classify_boundaries(&op, &roles).unwrap();
        assert_eq!(boundaries[1].text, "Runtime compatibility unverified");
        let conditions = boundaries
            .iter()
            .filter(|b| b.audience == BoundaryAudience::ReaderCondition)
            .map(|b| b.text.clone())
            .collect::<Vec<_>>();
        let mut section = human::operation(&op, &conditions);
        section.reference = "clew-reader-v1:opaque".into();
        document.sections.push(section);
        let html = human::render(&document).unwrap();
        for expected in [
            "Save `cityId`",
            "Errors keep the request pending",
            "Required string",
            "Delivery is not guaranteed",
            "<svg",
            "data-open-detail",
            "data-expand",
            "clew-reader-v1:opaque",
        ] {
            assert!(html.contains(expected), "missing {expected}");
        }
        for forbidden in [
            "PRIVATE_SOURCE_SENTINEL",
            "PRIVATE_BINDING_SENTINEL",
            "Runtime compatibility unverified",
            "sourceIds",
            "fragmentSources",
            "operationSources",
            "application/json",
            "bindings.json",
            "publication.json",
            "Human/imported prose is unassessed",
            "Retained static explanation; runtime behavior",
            "ordered static explanation, not a runtime trace",
            "current behavior is not reverified by this copy",
        ] {
            assert!(!html.contains(forbidden), "leaked {forbidden}");
        }
    }

    #[test]
    fn reference_binds_rendered_presentation_without_invalidating_old_version() {
        let (tmp, repo, saved) = fixture();
        let file = tmp.path().join("reader.html");
        fs::write(&file, "original export").unwrap();
        let old_reference = receipt(&repo, &saved, &file);
        let (mut record, _) = resolve(&repo, &old_reference, &file).unwrap();
        let mut view = human::Document::new("First presentation".into());
        view.sections.push(human::Section::markdown(
            "Section".into(),
            "Same original prose".into(),
        ));
        address_view(&mut record, &mut view).unwrap();
        let first = view.sections[0].reference.clone();
        let mut successor = human::Document::new("Revised presentation".into());
        successor.sections.push(human::Section::markdown(
            "Section".into(),
            "Same original prose".into(),
        ));
        address_view(&mut record, &mut successor).unwrap();
        let second = successor.sections[0].reference.clone();
        assert_ne!(first, second);
        address_view(&mut record, &mut successor).unwrap();
        assert_eq!(second, successor.sections[0].reference);
        // A new renderer address never replaces the original ledger entry.
        resolve(&repo, &old_reference, &file).unwrap();
    }
}
