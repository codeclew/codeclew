//! Human rendering and exact private lookup share an existing immutable Work.
//! The public view deliberately cannot serialize Source/Observation/Bindings.

use super::{bytes, cache, digest, invalid, io_error, model::*, store::Repository, work};
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
        json!({"kind":"SECTION","id":section.id,"origin":section.origin,"contentDigest":section.content_digest,"snapshot":record.snapshot,"subject":record.subject,"documentDigest":record.document_digest,"fileDigest":record.file_digest,"publication":record.publication,"renderPolicy":render_policy,"repairSupported":false,"repairBaseAvailable":record.source_map.is_some(),"repairAdapter":null,"repairLimit":"Historical source map only. Apply edits to authored originals through compose/bind/finalize; native export regeneration is retired"}),
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
        json!({"status":"EXACT_READER_REPAIR_BASE_EXPORTED","reference":reference,"claim":section.id,"old":section.text,"baseMarkdownDigest":hash_bytes(markdown.as_bytes()),"baseMapDigest":map["mapDigest"],"base":output,"next":"Original Markdown and source map exported. Review these exact originals and migrate explicitly through the compose/bind/finalize human pipeline; no meaning approval is implied.","unsupported":"List/table/code/link structural mutation; preserve existing guarded paragraph rules."}),
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
}
