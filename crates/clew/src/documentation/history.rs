//! Frozen publication manifests and offline history inspection.
use super::{
    bindings::{self, Bindings},
    digest, invalid, io_error,
    model::SectionState,
    store::{self, Repository},
};
use crate::{canonical, error::ClewError};
use clap::Subcommand;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::PathBuf,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Publication {
    pub schema: String,
    pub id: String,
    /// True only when the user explicitly released this immutable render.
    /// Older manifests omit the field and are treated as working snapshots.
    #[serde(default)]
    pub released: bool,
    pub parent: Option<String>,
    pub ordinal: u64,
    pub input_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub documentation_language: Option<String>,
    pub target_revisions: BTreeMap<String, Option<String>>,
    pub sections: BTreeMap<String, SectionState>,
    pub explanation_versions: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub reviewed_answers: BTreeMap<String, String>,
    pub observed_tags: BTreeMap<String, BTreeMap<String, String>>,
    pub evidence_packages: Vec<String>,
    pub files: BTreeMap<String, String>,
}
#[derive(Debug, Subcommand)]
pub enum Command {
    List {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        cursor: Option<String>,
        #[arg(long,default_value_t=20,value_parser=clap::value_parser!(u32).range(1..=100))]
        limit: u32,
    },
    Show {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        id: String,
        #[arg(long, default_value = "summary")]
        /// summary, sections, explanations, reviewed-answers, files, inputs,
        /// evidence, or bindings-closure (root-relative portable object paths).
        kind: String,
        #[arg(long)]
        cursor: Option<String>,
        #[arg(long,default_value_t=20,value_parser=clap::value_parser!(u32).range(1..=100))]
        limit: u32,
    },
    Compare {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        before: String,
        #[arg(long)]
        after: String,
        #[arg(long)]
        cursor: Option<String>,
        #[arg(long,default_value_t=20,value_parser=clap::value_parser!(u32).range(1..=100))]
        limit: u32,
    },
}
fn valid(id: &str) -> bool {
    id.len() == 64
        && id
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
fn path(id: &str) -> Result<String, ClewError> {
    if !valid(id) {
        return Err(invalid("invalid publication identity"));
    }
    Ok(format!("docs/generated/{id}/publication.json"))
}
pub(super) fn load(repo: &Repository, id: &str) -> Result<Publication, ClewError> {
    let p: Publication = store::read(
        &repo.path(&path(id)?)?,
        super::check::PORTABLE_CACHE_MAX_BYTES,
    )?;
    if p.schema != "codeclew-documentation-publication/1.0"
        || p.id != id
        || (p.released && p.ordinal == 0)
        || p.parent.as_ref().is_some_and(|p| !valid(p) || p == id)
        || p.documentation_language
            .as_deref()
            .is_some_and(|language| !matches!(language, "en" | "ru"))
        || p.files.len() > 8192
        || p.sections.len() > 32768
        || p.reviewed_answers.len() > 64
    {
        return Err(invalid("invalid frozen publication manifest"));
    }
    for name in p.files.keys() {
        store::relative(name)?;
    }
    Ok(p)
}
fn integrity(repo: &Repository, p: &Publication) -> Result<Vec<String>, ClewError> {
    let mut missing = Vec::new();
    for (name, expected) in &p.files {
        let file = repo.path(&format!("docs/generated/{}/{name}", p.id))?;
        match fs::metadata(&file) {
            Ok(m) if m.is_file() && m.len() <= super::check::PORTABLE_CACHE_MAX_BYTES => {
                let raw = fs::read(&file).map_err(io_error)?;
                if canonical::hash_bytes(&raw) != *expected {
                    missing.push(name.clone());
                } else if name == "bindings.json"
                    && super::bindings_storage::resolve_optional(repo, &raw).is_err()
                {
                    missing.push("bindings portable closure".into());
                }
            }
            _ => missing.push(name.clone()),
        }
    }
    Ok(missing)
}
/// Native-only cumulative verification budget; ordinary history admission is unchanged.
pub(super) struct FrozenInputBudget {
    pub remaining_bytes: u64,
}
fn embedded_limit_message(detail: &str) -> String {
    format!(
        "{detail}. Use the existing remote route with the original publication and exact snapshot: clew docs pages render --root ROOT --publication PUB --snapshot SNAP --source-mode remote --export-subject service:ID --source-map repos.json --output NEW_DIRECTORY (or scenario:ID; repeat --export-subject for several). --input cannot be combined with --publication. Remote export has separate bounds, includes shared context, and still rejects missing/invalid selected closure; it does not refresh evidence. See installed references/service-documentation.md: Source links for shared documentation."
    )
}
/// Read the exact immutable publication without loading any original Check.
pub(super) fn read_frozen_bindings(
    repo: &Repository,
    id: &str,
    budget: &mut FrozenInputBudget,
) -> Result<(Publication, Vec<u8>), ClewError> {
    use std::io::Read;
    let manifest = repo.path(&path(id)?)?;
    let size = fs::metadata(&manifest)
        .map_err(|_| invalid("selected frozen publication is unavailable"))?
        .len();
    if size > budget.remaining_bytes {
        return Err(invalid(embedded_limit_message(
            "native frozen publication input exceeds 64 MiB",
        )));
    }
    budget.remaining_bytes -= size;
    let publication = load(repo, id)?;
    // Preflight every file before hashing or allocating any content file.
    let mut total = 0u64;
    for name in publication.files.keys() {
        let metadata = fs::metadata(repo.path(&format!("docs/generated/{id}/{name}"))?)
            .map_err(|_| invalid("selected frozen publication is damaged or incomplete"))?;
        if !metadata.is_file() {
            return Err(invalid(
                "selected frozen publication is damaged or incomplete",
            ));
        }
        total = total
            .checked_add(metadata.len())
            .ok_or_else(|| invalid("native frozen input size overflow"))?;
        if total > budget.remaining_bytes {
            return Err(invalid(embedded_limit_message(
                "native frozen publication input exceeds 64 MiB",
            )));
        }
    }
    // The root manifest is bounded by the ordinary frozen-file preflight. Its
    // declared portable closure is charged before any payload hydration.
    let expected_bindings = publication
        .files
        .get("bindings.json")
        .ok_or_else(|| invalid("selected publication has no frozen bindings hash"))?;
    let binding_path = repo.path(&format!("docs/generated/{id}/bindings.json"))?;
    let mut binding_raw = Vec::new();
    fs::File::open(binding_path)
        .map_err(io_error)?
        .take(budget.remaining_bytes + 1)
        .read_to_end(&mut binding_raw)
        .map_err(io_error)?;
    if binding_raw.len() as u64 > budget.remaining_bytes {
        return Err(invalid(embedded_limit_message(
            "native frozen publication input exceeds 64 MiB",
        )));
    }
    if canonical::hash_bytes(&binding_raw) != *expected_bindings {
        return Err(invalid(
            "selected frozen publication is damaged or incomplete: bindings changed after preflight",
        ));
    }
    let closure = super::bindings_storage::closure(&binding_raw)?;
    let closure_bytes = closure.iter().map(|r| r.size).sum::<u64>();
    let physical_bindings = binding_raw.len() as u64 + closure_bytes;
    let logical_extra = super::bindings_storage::logical_size(&binding_raw)?
        .unwrap_or(0)
        .saturating_sub(physical_bindings);
    for reference in &closure {
        let metadata =
            fs::symlink_metadata(repo.path(&super::bindings_storage::object_path(reference)?)?)
                .map_err(|_| invalid("selected frozen publication portable closure is missing"))?;
        if !metadata.is_file() || metadata.len() != reference.size {
            return Err(invalid(
                "selected frozen publication portable closure is damaged",
            ));
        }
        total = total
            .checked_add(reference.size)
            .ok_or_else(|| invalid("native frozen input size overflow"))?;
        if total > budget.remaining_bytes {
            return Err(crate::error::ClewError::new(
                crate::error::ErrorCode::SliceBudgetExceeded,
                embedded_limit_message(
                    "native frozen publication input exceeds 64 MiB including its portable closure",
                ),
            ));
        }
    }
    total = total
        .checked_add(logical_extra)
        .ok_or_else(|| invalid("native frozen input size overflow"))?;
    if total > budget.remaining_bytes {
        return Err(crate::error::ClewError::new(
            crate::error::ErrorCode::SliceBudgetExceeded,
            embedded_limit_message(
                "native frozen publication input exceeds 64 MiB including reconstructed logical bindings",
            ),
        ));
    }
    // Hash verification uses the same portable reader; no source capture or
    // private snapshot loading occurs during this physical-input admission.
    let other_bytes = total
        .saturating_sub(physical_bindings)
        .saturating_sub(logical_extra);
    super::bindings_storage::preflight_frozen(
        repo,
        &binding_raw,
        budget.remaining_bytes.saturating_sub(other_bytes),
    )?;
    budget.remaining_bytes -= closure_bytes + logical_extra;
    let mut raw = None;
    for (name, expected) in &publication.files {
        let mut bytes = Vec::new();
        fs::File::open(repo.path(&format!("docs/generated/{id}/{name}"))?)
            .map_err(io_error)?
            .take(budget.remaining_bytes + 1)
            .read_to_end(&mut bytes)
            .map_err(io_error)?;
        if bytes.len() as u64 > budget.remaining_bytes {
            return Err(invalid(embedded_limit_message(
                "native frozen publication input exceeds 64 MiB",
            )));
        }
        budget.remaining_bytes -= bytes.len() as u64;
        if canonical::hash_bytes(&bytes) != *expected {
            return Err(invalid(
                "selected frozen publication is damaged or incomplete",
            ));
        }
        if name == "bindings.json" {
            raw = Some(bytes);
        }
    }
    Ok((
        publication,
        raw.ok_or_else(|| invalid("selected publication has no frozen bindings hash"))?,
    ))
}
/// Validation runs only after native preflight has bounded all original contexts.
pub(super) fn validate_frozen_bindings(
    repo: &Repository,
    publication: Publication,
    raw: &[u8],
    contexts: &mut BTreeMap<String, super::check::Check>,
) -> Result<(Publication, Bindings), ClewError> {
    let binding = bindings::validate_retained_bindings_with_contexts(repo, raw, contexts)?;
    validate_publication_binding(&publication, &binding)?;
    Ok((publication, binding))
}

fn validate_publication_binding(
    publication: &Publication,
    binding: &Bindings,
) -> Result<(), ClewError> {
    if publication.input_digest != binding.input_digest
        || publication.documentation_language != binding.documentation_language
    {
        return Err(invalid(
            "selected publication metadata does not match its frozen bindings",
        ));
    }
    if publication.reviewed_answers
        != binding
            .reviewed_answers
            .iter()
            .map(|(id, e)| Ok((id.clone(), digest(e)?)))
            .collect::<Result<BTreeMap<_, _>, ClewError>>()?
    {
        return Err(invalid(
            "frozen reviewed answer manifest differs from binding",
        ));
    }
    for (subject, narrative) in &binding.narratives {
        for operation in &narrative.operations {
            if publication
                .explanation_versions
                .get(&format!("{subject}/{}", operation.id))
                != Some(&digest(operation)?)
            {
                return Err(invalid(
                    "selected publication operation digest does not match its frozen bindings",
                ));
            }
        }
    }
    Ok(())
}
fn missing_packages(repo: &Repository, p: &Publication) -> Result<Vec<String>, ClewError> {
    p.evidence_packages
        .iter()
        .filter_map(|id| match super::evidence_package::retained(repo, id) {
            Ok(true) => None,
            Ok(false) => Some(Ok(id.clone())),
            Err(e) => Some(Err(e)),
        })
        .collect()
}
fn records(repo: &Repository) -> Result<Vec<Publication>, ClewError> {
    let root = repo.path("docs/generated")?;
    if !root.exists() {
        return Ok(vec![]);
    }
    let mut rows = Vec::new();
    let mut scanned = 0usize;
    for entry in fs::read_dir(root).map_err(io_error)? {
        let entry = entry.map_err(io_error)?;
        let id = entry.file_name().to_string_lossy().into_owned();
        if !valid(&id) {
            continue;
        }
        if scanned >= 4096 {
            return Err(invalid(
                "publication enumeration exceeds 4096 bundles; narrow retained history through the external retention policy",
            ));
        }
        scanned += 1;
        if repo.path(&path(&id)?)?.exists() {
            let publication = load(repo, &id)?;
            if publication.released {
                if rows.len() >= 4096 {
                    return Err(invalid(
                        "history enumeration exceeds 4096 released snapshots; narrow retained history through the external retention policy",
                    ));
                }
                rows.push(publication);
            }
        }
    }
    rows.sort_by(|a, b| b.ordinal.cmp(&a.ordinal).then(a.id.cmp(&b.id)));
    Ok(rows)
}

pub fn run(command: Command) -> Result<Value, ClewError> {
    match command {
        Command::List {
            root,
            cursor,
            limit,
        } => {
            let repo = Repository::open_portable(&root)?;
            let rows = records(&repo)?;
            let baseline = bindings::baseline(&repo)?.map(|b| b.0);
            let current =
                baseline.filter(|id| rows.iter().any(|publication| &publication.id == id));
            let latest_release = rows.first().map(|publication| publication.id.as_str());
            let binding = digest(&rows)?;
            super::cli::page(&binding,rows.iter().map(|p|json!({"id":p.id,"released":p.released,"ordinal":p.ordinal,"parent":p.parent,"current":current.as_ref()==Some(&p.id),"latestReleased":Some(p.id.as_str())==latest_release,"targets":p.target_revisions,"tags":p.observed_tags})).collect(),cursor.as_deref(),limit as usize,json!({"schema":"codeclew-docs-history-list/1.0"}))
        }
        Command::Show {
            root,
            id,
            kind,
            cursor,
            limit,
        } => {
            let repo = Repository::open_portable(&root)?;
            if !repo.path(&path(&id)?)?.exists() {
                return Ok(
                    json!({"schema":"codeclew-docs-history-show/1.0","status":"EXPIRED_OR_LEGACY_MANIFEST_MISSING","id":id}),
                );
            }
            let p = load(&repo, &id)?;
            let missing = integrity(&repo, &p)?;
            let packages = missing_packages(&repo, &p)?;
            let rows=match kind.as_str(){
            "summary"=>vec![json!({"id":p.id,"released":p.released,"ordinal":p.ordinal,"parent":p.parent,"inputDigest":p.input_digest,"targetRevisions":p.target_revisions,"observedTags":p.observed_tags})],
            "sections"=>p.sections.iter().map(|(id,state)|json!({"id":id,"state":state})).collect(),
            "explanations"=>p.explanation_versions.iter().map(|(id,version)|json!({"id":id,"version":version})).collect(),
            "reviewed-answers"=>p.reviewed_answers.iter().map(|(id,version)|json!({"id":id,"version":version})).collect(),
            "files"=>p.files.iter().map(|(id,hash)|json!({"id":id,"digest":hash})).collect(),
            "bindings-closure"=>{
                use std::io::Read;
                let mut raw=Vec::new();fs::File::open(repo.path(&format!("docs/generated/{id}/bindings.json"))?).map_err(io_error)?
                    .take(super::check::PORTABLE_CACHE_MAX_BYTES+1).read_to_end(&mut raw).map_err(io_error)?;
                super::bindings_storage::closure(&raw)?.into_iter().map(|r|Ok(json!({"id":r.digest,"path":super::bindings_storage::object_path(&r)?,"schema":r.schema,"size":r.size}))).collect::<Result<Vec<_>,ClewError>>()?
            },
            "inputs"=>{let value:Value=store::read(&repo.path(&format!("docs/generated/{id}/inputs.json"))?,super::check::PORTABLE_CACHE_MAX_BYTES)?;value.as_object().ok_or_else(||invalid("invalid frozen inputs"))?.iter().map(|(id,value)|json!({"id":id,"record":value})).collect()},
            "evidence"=>p.evidence_packages.iter().map(|id|json!({"id":id,"status":if packages.contains(id){"MISSING_OR_DAMAGED"}else{"RETAINED"}})).collect(),
            _=>return Err(invalid("unknown history record kind")),
        };
            super::cli::page(
                &digest(&(&p, &kind))?,
                rows,
                cursor.as_deref(),
                limit as usize,
                json!({"schema":"codeclew-docs-history-show/1.0","id":id,"status":if missing.is_empty(){"FROZEN_SNAPSHOT"}else{"DAMAGED_OR_EXPIRED"},"missingFiles":missing,"evidenceRetention":if packages.is_empty(){"COMPLETE"}else{"MISSING_PACKAGES"}}),
            )
        }
        Command::Compare {
            root,
            before,
            after,
            cursor,
            limit,
        } => {
            let repo = Repository::open_portable(&root)?;
            let a = load(&repo, &before)?;
            let b = load(&repo, &after)?;
            if !integrity(&repo, &a)?.is_empty() || !integrity(&repo, &b)?.is_empty() {
                return Err(invalid(
                    "cannot compare damaged or expired publication files",
                ));
            }
            let keys: BTreeSet<_> = a.sections.keys().chain(b.sections.keys()).collect();
            let mut rows = Vec::new();
            for key in keys {
                let left = a.sections.get(key).map(|v| json!(v));
                let right = b.sections.get(key).map(|v| json!(v));
                if left != right {
                    rows.push(json!({"id":key,"before":left,"after":right}));
                }
            }
            if a.observed_tags != b.observed_tags {
                rows.push(
                    json!({"id":"observed-tags","before":a.observed_tags,"after":b.observed_tags}),
                );
            }
            for id in a
                .explanation_versions
                .keys()
                .chain(b.explanation_versions.keys())
                .collect::<BTreeSet<_>>()
            {
                let before = a.explanation_versions.get(id);
                let after = b.explanation_versions.get(id);
                if before != after {
                    rows.push(
                        json!({"id":format!("{id}/explanation"),"before":before,"after":after}),
                    );
                }
            }
            super::cli::page(
                &digest(&(&a, &b))?,
                rows,
                cursor.as_deref(),
                limit as usize,
                json!({"schema":"codeclew-docs-history-compare/1.0","before":before,"after":after,"beforeTargets":a.target_revisions,"afterTargets":b.target_revisions}),
            )
        }
    }
}

#[cfg(test)]
mod language_tests {
    use super::super::bytes;
    use super::*;
    #[test]
    fn frozen_manifest_budget_refuses_before_parsing_and_keeps_recovery_exact() {
        let temporary = tempfile::tempdir().unwrap();
        Repository::init(temporary.path(), "Docs").unwrap();
        let repo = Repository::open(temporary.path()).unwrap();
        let id = "a".repeat(64);
        repo.atomic(&path(&id).unwrap(), b"x").unwrap();
        let mut budget = FrozenInputBudget { remaining_bytes: 0 };
        let error = read_frozen_bindings(&repo, &id, &mut budget).unwrap_err();
        assert_eq!(error.code, crate::error::ErrorCode::InvalidInput);
        assert_eq!(budget.remaining_bytes, 0);
        assert!(error.message.contains("--publication PUB --snapshot SNAP"));
        assert!(error.message.contains("--export-subject"));
        // Fitting the manifest reaches ordinary validation, not an export fallback.
        budget.remaining_bytes = 1;
        let error = read_frozen_bindings(&repo, &id, &mut budget).unwrap_err();
        assert!(!error.message.contains("exceeds 64 MiB"));
        assert_eq!(budget.remaining_bytes, 0);
        assert_eq!(
            fs::read(repo.path(&path(&id).unwrap()).unwrap()).unwrap(),
            b"x"
        );
    }
    #[test]
    fn working_snapshots_explain_empty_history_without_becoming_releases() {
        let root = tempfile::tempdir().unwrap();
        Repository::init(root.path(), "Docs").unwrap();
        let repo = Repository::open(root.path()).unwrap();
        let id = "a".repeat(64);
        let working: Publication = serde_json::from_value(json!({
            "schema":"codeclew-documentation-publication/1.0","id":id,
            "released":false,"parent":null,"ordinal":0,"inputDigest":"digest",
            "documentationLanguage":"en","targetRevisions":{},"sections":{},
            "explanationVersions":{},"observedTags":{},"evidencePackages":[],"files":{}
        }))
        .unwrap();
        let original = bytes(&working).unwrap();
        repo.atomic(&path(&id).unwrap(), &original).unwrap();

        assert!(records(&repo).unwrap().is_empty());
        assert_eq!(
            fs::read(repo.path(&path(&id).unwrap()).unwrap()).unwrap(),
            original
        );

        let mut released = working;
        released.id = "b".repeat(64);
        released.released = true;
        released.ordinal = 1;
        repo.atomic(&path(&released.id).unwrap(), &bytes(&released).unwrap())
            .unwrap();

        assert_eq!(records(&repo).unwrap().len(), 1);
        assert_eq!(
            fs::read(repo.path(&path(&id).unwrap()).unwrap()).unwrap(),
            original
        );
    }

    #[test]
    fn history_localizes_chrome_and_preserves_legacy_language_absence() {
        let id = "a".repeat(64);
        let legacy = json!({"schema":"codeclew-documentation-publication/1.0","id":id,"parent":null,"ordinal":1,"inputDigest":"digest","targetRevisions":{},"sections":{},"explanationVersions":{},"observedTags":{},"evidencePackages":[],"files":{}});
        let legacy_publication: Publication = serde_json::from_value(legacy.clone()).unwrap();
        assert!(!legacy_publication.released);
        assert!(legacy_publication.documentation_language.is_none());
        let legacy_serialized = serde_json::to_value(&legacy_publication).unwrap();
        assert_eq!(legacy_serialized["released"], false);
        assert!(legacy_serialized.get("documentationLanguage").is_none());

        let schema: Value = serde_json::from_str(include_str!(
            "../../../../schemas/documentation/publication.schema.json"
        ))
        .unwrap();
        assert_eq!(schema["properties"]["released"]["type"], "boolean");
        assert_eq!(
            schema["properties"]["documentationLanguage"]["enum"],
            json!(["en", "ru"])
        );
        assert_eq!(schema["properties"]["ordinal"]["minimum"], 0);
        assert_eq!(
            schema["allOf"][0]["if"]["properties"]["released"]["const"],
            true
        );
        assert_eq!(
            schema["allOf"][0]["then"]["properties"]["ordinal"]["minimum"],
            1
        );
        assert!(
            schema["required"]
                .as_array()
                .unwrap()
                .iter()
                .all(|name| { name != "released" && name != "documentationLanguage" })
        );

        let root = tempfile::tempdir().unwrap();
        Repository::init(root.path(), "Docs").unwrap();
        let repo = Repository::open(root.path()).unwrap();
        repo.atomic(&path(&id).unwrap(), &bytes(&legacy_publication).unwrap())
            .unwrap();
        assert!(!load(&repo, &id).unwrap().released);

        let mut language_unspecified = legacy_publication.clone();
        language_unspecified.id = "c".repeat(64);
        language_unspecified.released = true;
        language_unspecified.ordinal = 1;
        repo.atomic(
            &path(&language_unspecified.id).unwrap(),
            &bytes(&language_unspecified).unwrap(),
        )
        .unwrap();

        let mut publication = legacy_publication;
        publication.id = "b".repeat(64);
        publication.released = true;
        publication.parent = Some(language_unspecified.id.clone());
        publication.ordinal = 2;
        publication.documentation_language = Some("ru".into());
        repo.atomic(
            &path(&publication.id).unwrap(),
            &bytes(&publication).unwrap(),
        )
        .unwrap();

        assert_eq!(
            serde_json::to_value(load(&repo, &id).unwrap()).unwrap(),
            legacy_serialized
        );
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct PublicationReceipt {
    pub schema: String,
    pub bundle_id: String,
    pub root_index_hash: String,
    pub bindings_hash: String,
    pub publication_hash: String,
    pub effective_gaps: BTreeMap<String, BTreeMap<String, String>>,
    pub requested_language: Option<String>,
    /// UI language used when the request and retained binding omit one.
    pub effective_language: String,
}
