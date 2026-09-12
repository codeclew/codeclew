//! Frozen publication manifests and offline history inspection.
use super::{
    bindings::{self, Bindings},
    bytes, digest, invalid, io_error,
    model::SectionState,
    render,
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
    pub parent: Option<String>,
    pub ordinal: u64,
    pub input_digest: String,
    pub target_revisions: BTreeMap<String, Option<String>>,
    pub sections: BTreeMap<String, SectionState>,
    pub explanation_versions: BTreeMap<String, String>,
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
fn load(repo: &Repository, id: &str) -> Result<Publication, ClewError> {
    let p: Publication = store::read(&repo.path(&path(id)?)?, 64 * 1024 * 1024)?;
    if p.schema != "codeclew-documentation-publication/1.0"
        || p.id != id
        || p.ordinal == 0
        || p.parent.as_ref().is_some_and(|p| !valid(p) || p == id)
        || p.files.len() > 8192
        || p.sections.len() > 32768
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
            Ok(m) if m.is_file() && m.len() <= 64 * 1024 * 1024 => {
                if canonical::hash_bytes(&fs::read(&file).map_err(io_error)?) != *expected {
                    missing.push(name.clone());
                }
            }
            _ => missing.push(name.clone()),
        }
    }
    Ok(missing)
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
    for entry in fs::read_dir(root).map_err(io_error)? {
        let entry = entry.map_err(io_error)?;
        let id = entry.file_name().to_string_lossy().into_owned();
        if !valid(&id) {
            continue;
        }
        if rows.len() >= 4096 {
            return Err(invalid(
                "history enumeration exceeds 4096 snapshots; narrow retained history through the external retention policy",
            ));
        }
        if repo.path(&path(&id)?)?.exists() {
            rows.push(load(repo, &id)?);
        }
    }
    rows.sort_by(|a, b| b.ordinal.cmp(&a.ordinal).then(a.id.cmp(&b.id)));
    Ok(rows)
}
fn nav(id: &str, parent: Option<&str>, nested: bool) -> String {
    let history = if nested {
        "../../../history.html"
    } else {
        "../../history.html"
    };
    let previous = parent
        .map(|p| {
            format!(
                "<a href=\"{}{p}/overview.html\">Previous snapshot</a>",
                if nested { "../../" } else { "../" }
            )
        })
        .unwrap_or_default();
    format!(
        "<nav aria-label=\"Snapshot history\" style=\"display:flex;gap:20px;flex-wrap:wrap;padding:12px 20px;background:#eff4e8;color:#263d2d;font:14px system-ui\"><a href=\"{history}\">Snapshot history</a><span>Frozen snapshot {}</span>{previous}</nav>",
        &id[..12]
    )
}
pub(super) fn prepare(
    repo: &Repository,
    id: &str,
    binding: &Bindings,
    files: &mut BTreeMap<String, Vec<u8>>,
    input_digest: &str,
    previous: Option<&(String, Bindings)>,
) -> Result<Publication, ClewError> {
    let existing = repo.path(&path(id)?)?;
    let retained = if existing.exists() {
        Some(load(repo, id)?)
    } else {
        None
    };
    let parent = retained
        .as_ref()
        .map(|p| p.parent.clone())
        .unwrap_or_else(|| previous.map(|p| p.0.clone()).filter(|p| p != id));
    let ordinal = if let Some(p) = &retained {
        p.ordinal
    } else {
        parent
            .as_ref()
            .and_then(|p| load(repo, p).ok())
            .map_or(1, |p| p.ordinal.saturating_add(1))
    };
    for (name, contents) in files
        .iter_mut()
        .filter(|(n, _)| n.ends_with(".html") && n.as_str() != "root-overview.html")
    {
        let nested = name.contains('/');
        let mut html = String::from_utf8(contents.clone()).map_err(io_error)?;
        if nested {
            html = html.replace("href=\"../../../index.html\"", "href=\"../overview.html\"");
        }
        html = html.replacen(
            "<body>",
            &format!("<body>{}", nav(id, parent.as_deref(), nested)),
            1,
        );
        *contents = html.into_bytes();
    }
    let declarations = json!({"schema":"codeclew-documentation-publication-inputs/1.0","manifest":repo.manifest,"services":repo.services()?,"interactions":repo.interactions()?,"scenarios":repo.scenarios()?,"entities":super::entities::records(repo)?,"notes":super::notes::snapshot(repo)?,"coordinator":super::updates::state(repo)?});
    files.insert("inputs.json".into(), bytes(&declarations)?);
    let mut packages = BTreeSet::new();
    for o in binding.observations.values().chain(
        binding
            .fragments
            .values()
            .filter_map(|f| f.evidence.as_ref())
            .flat_map(|e| e.observations.values()),
    ) {
        if o.kind == "EVIDENCE_PACKAGE"
            && let Some(id) = o.normalized["expectation"]["manifestDigest"].as_str()
        {
            packages.insert(id.to_owned());
        }
    }
    let mut tags: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    for (id, event) in super::updates::state(repo)?.targets {
        if let Some(tag) = event.tag {
            tags.entry(id).or_default().insert(tag, event.revision);
        }
    }
    Ok(Publication {
        schema: "codeclew-documentation-publication/1.0".into(),
        id: id.into(),
        parent,
        ordinal,
        input_digest: input_digest.into(),
        target_revisions: binding.target_revisions.clone(),
        sections: binding.section_states.clone(),
        explanation_versions: binding
            .narratives
            .iter()
            .flat_map(|(subject, n)| {
                n.operations
                    .iter()
                    .map(move |o| (format!("{subject}/{}", o.id), o))
            })
            .map(|(id, o)| Ok((id, digest(o)?)))
            .collect::<Result<_, ClewError>>()?,
        observed_tags: tags,
        evidence_packages: packages.into_iter().collect(),
        files: BTreeMap::new(),
    })
}
pub(super) fn index(repo: &Repository, current: &str) -> Result<(), ClewError> {
    let rows = records(repo)?;
    let cards=rows.iter().map(|p|format!("<li><a href=\"generated/{}/overview.html\">Snapshot {} · {}</a>{}<details><summary>Observed targets and tags</summary><pre>{}</pre></details></li>",p.id,p.ordinal,&p.id[..12],if p.id==current{" · Current publication"}else{""},render::escape(&json!({"targets":p.target_revisions,"tags":p.observed_tags}).to_string()))).collect::<String>();
    repo.atomic("docs/history.html",format!("<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>Documentation history</title><style>pre{{white-space:pre-wrap;overflow-wrap:anywhere}}li{{margin-block:16px}}a{{overflow-wrap:anywhere}}</style></head><body style=\"font:16px system-ui;max-width:1000px;margin:auto;padding:24px\"><h1>Documentation history</h1><p><a href=\"index.html\">Current publication</a></p><p>Snapshots preserve their observed revisions and meaning review. A moved tag does not rewrite a snapshot. Use history inspection to verify retained files and evidence availability.</p><ol>{cards}</ol></body></html>").as_bytes())
}
pub fn run(command: Command) -> Result<Value, ClewError> {
    match command {
        Command::List {
            root,
            cursor,
            limit,
        } => {
            let repo = Repository::open(&root)?;
            let rows = records(&repo)?;
            let current = bindings::baseline(&repo)?.map(|b| b.0);
            let binding = digest(&rows)?;
            super::cli::page(&binding,rows.iter().map(|p|json!({"id":p.id,"ordinal":p.ordinal,"parent":p.parent,"current":current.as_ref()==Some(&p.id),"targets":p.target_revisions,"tags":p.observed_tags})).collect(),cursor.as_deref(),limit as usize,json!({"schema":"codeclew-docs-history-list/1.0"}))
        }
        Command::Show {
            root,
            id,
            kind,
            cursor,
            limit,
        } => {
            let repo = Repository::open(&root)?;
            if !repo.path(&path(&id)?)?.exists() {
                return Ok(
                    json!({"schema":"codeclew-docs-history-show/1.0","status":"EXPIRED_OR_LEGACY_MANIFEST_MISSING","id":id}),
                );
            }
            let p = load(&repo, &id)?;
            let missing = integrity(&repo, &p)?;
            let packages = missing_packages(&repo, &p)?;
            let rows=match kind.as_str(){
            "summary"=>vec![json!({"id":p.id,"ordinal":p.ordinal,"parent":p.parent,"inputDigest":p.input_digest,"targetRevisions":p.target_revisions,"observedTags":p.observed_tags})],
            "sections"=>p.sections.iter().map(|(id,state)|json!({"id":id,"state":state})).collect(),
            "explanations"=>p.explanation_versions.iter().map(|(id,version)|json!({"id":id,"version":version})).collect(),
            "files"=>p.files.iter().map(|(id,hash)|json!({"id":id,"digest":hash})).collect(),
            "inputs"=>{let value:Value=store::read(&repo.path(&format!("docs/generated/{id}/inputs.json"))?,64*1024*1024)?;value.as_object().ok_or_else(||invalid("invalid frozen inputs"))?.iter().map(|(id,value)|json!({"id":id,"record":value})).collect()},
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
            let repo = Repository::open(&root)?;
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
