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
fn nav(id: &str, parent: Option<&str>, nested: bool, language: &str) -> String {
    let label = |en, ru| super::reader::text(language, en, ru);
    let previous_label = label("Previous snapshot", "Предыдущий снимок");
    let snapshot = label("Snapshot", "Снимок");
    let retained = label(
        "retained source version",
        "сохранённая версия исходных данных",
    );
    let history_label = label("Snapshot history", "История снимков");
    let history = if nested {
        "../../../history.html"
    } else {
        "../../history.html"
    };
    let previous = parent
        .map(|p| {
            format!(
                "<a href=\"{}{p}/overview.html\">{previous_label}</a>",
                if nested { "../../" } else { "../" }
            )
        })
        .unwrap_or_default();
    format!(
        "<details class=\"snapshot-history\"><summary>{snapshot} {} · {retained}</summary><nav class=\"snapshot-links\" aria-label=\"{history_label}\"><a href=\"{history}\">{history_label}</a>{previous}</nav></details>",
        &id[..12]
    )
}
pub(super) fn prepare(
    repo: &Repository,
    id: &str,
    binding: &Bindings,
    files: &mut BTreeMap<String, Vec<u8>>,
    input_digest: &str,
    released: bool,
) -> Result<Publication, ClewError> {
    let existing = repo.path(&path(id)?)?;
    let retained = if existing.exists() {
        Some(load(repo, id)?)
    } else {
        None
    };
    if retained.as_ref().is_some_and(|p| p.released != released) {
        return Err(invalid(
            "publication identity conflicts with its existing release mode",
        ));
    }
    let (parent, ordinal) = if !released {
        (None, 0)
    } else if let Some(p) = &retained {
        (p.parent.clone(), p.ordinal)
    } else {
        let parent = records(repo)?.into_iter().next();
        match parent {
            Some(publication) => (Some(publication.id), publication.ordinal.saturating_add(1)),
            None => (None, 1),
        }
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
        if released {
            html = html.replacen(
                "</nav>",
                &format!(
                    "</nav>{}",
                    nav(
                        id,
                        parent.as_deref(),
                        nested,
                        binding.documentation_language.as_deref().unwrap_or("en")
                    )
                ),
                1,
            );
        }
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
        released,
        parent,
        ordinal,
        input_digest: input_digest.into(),
        documentation_language: binding.documentation_language.clone(),
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
    let publication = load(repo, current)?;
    let language = publication
        .documentation_language
        .as_deref()
        .unwrap_or("en");
    let label = |en, ru| super::reader::text(language, en, ru);
    let snapshot = label("Snapshot", "Снимок");
    let latest_release_label = label("Latest release", "Последний выпуск");
    let targets = label("Observed targets and tags", "Сохранённые версии и метки");
    let latest_release = rows.first().map(|p| p.id.as_str());
    let cards = rows.iter().map(|p| {
        let publication_language = match p.documentation_language.as_deref() {
            Some("en") => label("English", "Английский"),
            Some("ru") => label("Russian", "Русский"),
            _ => label("Original version (language unspecified)", "Исходная версия (язык не указан)"),
        };
        format!("<li><a href=\"generated/{}/overview.html\">{snapshot} {} · {}</a> · {}{}<details><summary>{targets}</summary><pre>{}</pre></details></li>", p.id, p.ordinal, &p.id[..12], publication_language, if Some(p.id.as_str()) == latest_release { format!(" · {latest_release_label}") } else { String::new() }, render::escape(&json!({"targets":p.target_revisions,"tags":p.observed_tags}).to_string()))
    }).collect::<String>();
    let navigation_publication = if publication.released {
        Some(&publication)
    } else {
        rows.first()
    };
    let (prefix, overview, pages) = navigation_publication
        .map(|p| {
            (
                format!("generated/{}/", p.id),
                "index.html".to_owned(),
                p.files.keys().cloned().collect::<Vec<_>>(),
            )
        })
        .unwrap_or_else(|| (String::new(), "index.html".to_owned(), Vec::new()));
    let nav = super::reader::navigation_language(&prefix, &overview, None, &pages, language);
    let title = label("Documentation history", "История документации");
    let explanation = label(
        "Snapshots preserve their observed revisions and meaning review. A moved tag does not rewrite a snapshot. Use history inspection to verify retained files and evidence availability.",
        "Снимки сохраняют версии исходных данных и состояние проверки смысла. Перемещение метки не изменяет снимок. Проверка истории позволяет убедиться в целостности сохранённых файлов и доступности подтверждающих данных.",
    );
    let body = format!("<h1>{title}</h1><p>{explanation}</p><ol>{cards}</ol>");
    repo.atomic(
        "docs/history.html",
        super::reader::decorate(&super::reader::page_language(title, &body, language), &nav)
            .as_bytes(),
    )
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
            "summary"=>vec![json!({"id":p.id,"released":p.released,"ordinal":p.ordinal,"parent":p.parent,"inputDigest":p.input_digest,"targetRevisions":p.target_revisions,"observedTags":p.observed_tags})],
            "sections"=>p.sections.iter().map(|(id,state)|json!({"id":id,"state":state})).collect(),
            "explanations"=>p.explanation_versions.iter().map(|(id,version)|json!({"id":id,"version":version})).collect(),
            "files"=>p.files.iter().map(|(id,hash)|json!({"id":id,"digest":hash})).collect(),
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

#[cfg(test)]
mod language_tests {
    use super::*;
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
        index(&repo, &publication.id).unwrap();
        let html = fs::read_to_string(repo.path("docs/history.html").unwrap()).unwrap();
        assert!(html.contains("<html lang=\"ru\">"));
        assert!(html.contains("История документации"));
        assert!(html.contains("Исходная версия (язык не указан)"));
        assert!(html.contains("Русский"));
        assert_eq!(
            serde_json::to_value(load(&repo, &id).unwrap()).unwrap(),
            legacy_serialized
        );
        assert!(
            nav(&publication.id, Some(&language_unspecified.id), true, "ru")
                .contains(&format!("../../{}/overview.html", language_unspecified.id))
        );
        assert!(
            nav(&publication.id, Some(&language_unspecified.id), true, "ru")
                .contains("Предыдущий снимок")
        );
    }
}
