//! Link existing service explanations without synthesizing their internal narrative.
//! A preview is a declared composition, never a runtime routing or call-graph proof.
use super::{invalid, io_error, operation_answer, render::escape, store};
use crate::{canonical::hash_bytes, error::ClewError};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    os::unix::fs::OpenOptionsExt,
    path::{Component, Path, PathBuf},
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Input {
    schema: String,
    title: String,
    scope: String,
    processes: Vec<Process>,
    steps: Vec<Step>,
}
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Process {
    pub id: String,
    pub title: String,
    pub service: String,
    pub revision: String,
    pub binding: Binding,
}
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(
    tag = "kind",
    rename_all = "SCREAMING_SNAKE_CASE",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Binding {
    CataloguePage {
        directory: PathBuf,
        export_digest: String,
        route: String,
        page_digest: String,
        operation: String,
        operation_digest: String,
    },
    NativeScenarioPage {
        directory: PathBuf,
        publication_digest: String,
        scenario: String,
        scenario_digest: String,
    },
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Step {
    id: String,
    process: String,
    #[serde(default)]
    anchor: Option<String>,
    #[serde(default)]
    children: Vec<Step>,
}
struct Linked {
    href: String,
    html: String,
    proof: Value,
}
/// Native threads reuse the same frozen-page pins as the linked preview.
/// The page stays at its original address; its narrative is never regenerated.
pub(super) fn project_link(process: &Process, budget: &mut u64) -> Result<Value, ClewError> {
    if !store::valid_id(&process.id)
        || !store::valid_id(&process.service)
        || !bounded(&process.title)
        || !bounded(&process.revision)
    {
        return Err(invalid("invalid frozen process page identity"));
    }
    let linked = match &process.binding {
        Binding::CataloguePage { .. } => catalogue_page(process, &process.binding, budget)?,
        Binding::NativeScenarioPage { .. } => native_scenario(process, &process.binding, budget)?,
    };
    Ok(
        json!({"id":process.id,"title":process.title,"service":process.service,
        "revision":process.revision,"href":linked.href,"proof":linked.proof,
        "internalNarrativeGeneration":false}),
    )
}
fn read(path: &Path, budget: &mut u64) -> Result<Vec<u8>, ClewError> {
    let mut file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(io_error)?;
    if !file.metadata().map_err(io_error)?.is_file() {
        return Err(invalid("preview input must be a regular file"));
    }
    let mut raw = Vec::new();
    file.by_ref()
        .take(*budget + 1)
        .read_to_end(&mut raw)
        .map_err(io_error)?;
    if raw.len() as u64 > *budget {
        return Err(invalid("linked preview exceeds its 64 MiB input budget"));
    }
    *budget -= raw.len() as u64;
    Ok(raw)
}
fn pinned(path: &Path, expected: &str, budget: &mut u64) -> Result<Vec<u8>, ClewError> {
    let raw = read(path, budget)?;
    if hash_bytes(&raw) != expected {
        return Err(invalid(
            "linked preview input changed from its selected digest",
        ));
    }
    Ok(raw)
}
fn decode(raw: &[u8]) -> Result<Value, ClewError> {
    serde_json::from_slice(raw).map_err(|_| invalid("invalid preview reference JSON"))
}
fn bounded(s: &str) -> bool {
    !s.trim().is_empty() && s.len() <= 4096
}
fn start(title: &str) -> String {
    format!(
        "<!doctype html><html lang=\"ru\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{}</title><style>{}</style></head><body><main><h1>{}</h1>",
        escape(title),
        operation_answer::OFFLINE_STYLE,
        escape(title)
    )
}
fn file_href(path: &Path) -> Result<String, ClewError> {
    let path = fs::canonicalize(path).map_err(io_error)?;
    let path = path
        .to_str()
        .ok_or_else(|| invalid("preview page path is not UTF-8"))?;
    let mut href = String::from("file://");
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~".contains(&byte) {
            href.push(byte as char);
        } else {
            href.push_str(&format!("%{byte:02X}"));
        }
    }
    Ok(href)
}
fn catalogue_page(
    process: &Process,
    binding: &Binding,
    budget: &mut u64,
) -> Result<Linked, ClewError> {
    let Binding::CataloguePage {
        directory,
        export_digest,
        route,
        page_digest,
        operation,
        operation_digest,
    } = binding
    else {
        unreachable!()
    };
    if !route.starts_with("processes/")
        || !route.ends_with(".html")
        || Path::new(route)
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
        || Path::new(route).components().count() != 2
    {
        return Err(invalid(
            "preview route must select one existing process page",
        ));
    }
    let report = decode(&pinned(
        &directory.join("export.json"),
        export_digest,
        budget,
    )?)?;
    if report["schema"] != "codeclew-documentation-catalogue-export/1.0" {
        return Err(invalid("unsupported process export reference"));
    }
    let rows: Vec<_> = report["processes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| r["route"] == *route)
        .collect();
    let [row] = rows.as_slice() else {
        return Err(invalid(
            "canonical process reference is absent or ambiguous",
        ));
    };
    if row["subject"] != format!("service:{}", process.service)
        || row["operation"] != *operation
        || row["operationDigest"] != *operation_digest
        || row["acceptedVersion"]["sourceRevisions"][&process.service] != process.revision
        || !matches!(
            row["acceptedVersion"]["verification"].as_str(),
            Some("VERIFIED" | "VERIFIED_WITH_LIMITATIONS")
        )
    {
        return Err(invalid(
            "canonical process reference disagrees with its accepted service/revision/operation",
        ));
    }
    let path = directory.join(route);
    let html = String::from_utf8(pinned(&path, page_digest, budget)?)
        .map_err(|_| invalid("canonical page is not UTF-8"))?;
    Ok(Linked {
        href: file_href(&path)?,
        html,
        proof: json!({"kind":"EXISTING_CANONICAL_PAGE","path":path,"pageDigest":page_digest,"exportDigest":export_digest,"operation":operation,"operationDigest":operation_digest,"acceptedVersion":row["acceptedVersion"]}),
    })
}
fn native_scenario(
    process: &Process,
    binding: &Binding,
    budget: &mut u64,
) -> Result<Linked, ClewError> {
    let Binding::NativeScenarioPage {
        directory,
        publication_digest,
        scenario,
        scenario_digest,
    } = binding
    else {
        unreachable!()
    };
    if !store::valid_id(scenario) {
        return Err(invalid("invalid native process scenario identity"));
    }
    let publication = decode(&pinned(
        &directory.join("publication.json"),
        publication_digest,
        budget,
    )?)?;
    let route = format!("scenarios/{scenario}.html");
    let json_route = format!("scenarios/{scenario}.json");
    if publication["files"][&json_route] != *scenario_digest {
        return Err(invalid(
            "native scenario differs from its publication manifest",
        ));
    }
    let page_digest = publication["files"][&route]
        .as_str()
        .ok_or_else(|| invalid("native process page is absent from publication"))?;
    let data = decode(&pinned(
        &directory.join(&json_route),
        scenario_digest,
        budget,
    )?)?;
    let thread = &data["processThread"];
    let root = thread["rootMethod"]
        .as_str()
        .ok_or_else(|| invalid("native process root is unavailable"))?;
    if data["subject"] != format!("scenario:{scenario}")
        || data["revisions"][&process.service] != process.revision
        || thread["status"] != "SOURCE_BOUND_DECLARATION"
        || thread["methods"][root]["source"]["service"] != process.service
        || thread["methods"][root]["source"]["revision"] != process.revision
    {
        return Err(invalid(
            "native process page differs from its selected root/service/revision",
        ));
    }
    let path = directory.join(&route);
    let html = String::from_utf8(pinned(&path, page_digest, budget)?)
        .map_err(|_| invalid("native process page is not UTF-8"))?;
    Ok(Linked {
        href: file_href(&path)?,
        html,
        proof: json!({"kind":"EXISTING_NATIVE_PROCESS_PAGE","path":path,"pageDigest":page_digest,"scenarioDigest":scenario_digest,"publicationDigest":publication_digest,"pageDigest":page_digest,"authority":thread["authority"],"internalNarrativeGeneration":false}),
    })
}
fn steps(
    rows: &[Step],
    processes: &BTreeMap<&str, &Process>,
    links: &BTreeMap<&str, Linked>,
    ids: &mut BTreeSet<String>,
    count: &mut usize,
    depth: usize,
) -> Result<String, ClewError> {
    if depth > 16 {
        return Err(invalid("linked preview nesting exceeds 16"));
    }
    let mut html = String::from("<ul class=\"nested-steps\">");
    for row in rows {
        *count += 1;
        let process = processes
            .get(row.process.as_str())
            .ok_or_else(|| invalid("linked step process is unavailable"))?;
        if *count > 128 || !store::valid_id(&row.id) || !ids.insert(row.id.clone()) {
            return Err(invalid("invalid or duplicate linked step identity"));
        }
        let link = &links[row.process.as_str()];
        let mut href = link.href.clone();
        if let Some(anchor) = &row.anchor {
            if !store::valid_id(anchor)
                || link.html.matches(&format!("id=\"{anchor}\"")).count() != 1
            {
                return Err(invalid(
                    "linked step anchor is absent or ambiguous in its canonical page",
                ));
            }
            href.push('#');
            href.push_str(anchor);
        }
        html.push_str(&operation_answer::step_node_open(
            &row.id,
            Some(&process.service),
        ));
        html.push_str(&format!(
            "<span class=\"step-text\"><a href=\"{}\">{} \u{2197}</a></span><p class=\"step-meta\">{}</p>",
            escape(&href),
            escape(&process.title),
            escape(&process.revision)
        ));
        if !row.children.is_empty() {
            html.push_str(&steps(
                &row.children,
                processes,
                links,
                ids,
                count,
                depth + 1,
            )?);
        }
        html.push_str("</div></li>");
    }
    html.push_str("</ul>");
    Ok(html)
}
pub(super) fn render(input: &Path, output: &Path) -> Result<Value, ClewError> {
    if output.exists() {
        return Err(invalid("linked preview requires a new output directory"));
    }
    let mut budget = 64 * 1024 * 1024;
    let raw = read(input, &mut budget)?;
    let spec: Input = serde_json::from_slice(&raw)
        .map_err(|_| invalid("invalid closed linked preview contract"))?;
    if spec.schema != "codeclew-linked-process-preview/1.0"
        || !bounded(&spec.title)
        || !bounded(&spec.scope)
        || spec.processes.len() < 2
        || spec.processes.len() > 32
        || spec.steps.is_empty()
    {
        return Err(invalid("invalid linked preview bounds"));
    }
    let mut processes = BTreeMap::new();
    let mut links = BTreeMap::new();
    for process in &spec.processes {
        if !store::valid_id(&process.id)
            || !store::valid_id(&process.service)
            || !bounded(&process.title)
            || !bounded(&process.revision)
            || processes.insert(process.id.as_str(), process).is_some()
        {
            return Err(invalid("invalid linked process identity"));
        }
        let linked = match &process.binding {
            Binding::CataloguePage { .. } => {
                catalogue_page(process, &process.binding, &mut budget)?
            }
            Binding::NativeScenarioPage { .. } => {
                native_scenario(process, &process.binding, &mut budget)?
            }
        };
        links.insert(process.id.as_str(), linked);
    }
    let mut html = start(&spec.title);
    html.push_str(&format!("<p>{}</p><p class=\"muted\">\u{41e}\u{431}\u{44a}\u{44f}\u{432}\u{43b}\u{435}\u{43d}\u{43d}\u{430}\u{44f} \u{441}\u{432}\u{44f}\u{437}\u{44c} \u{432}\u{44b}\u{431}\u{440}\u{430}\u{43d}\u{43d}\u{44b}\u{445} \u{441}\u{435}\u{440}\u{432}\u{438}\u{441}\u{43d}\u{44b}\u{445} \u{441}\u{442}\u{440}\u{430}\u{43d}\u{438}\u{446}. \u{421}\u{441}\u{44b}\u{43b}\u{43a}\u{438} \u{43e}\u{442}\u{43a}\u{440}\u{44b}\u{432}\u{430}\u{44e}\u{442} \u{441}\u{443}\u{449}\u{435}\u{441}\u{442}\u{432}\u{443}\u{44e}\u{449}\u{438}\u{435} \u{43f}\u{43e}\u{44f}\u{441}\u{43d}\u{435}\u{43d}\u{438}\u{44f}. \u{412}\u{43d}\u{443}\u{442}\u{440}\u{435}\u{43d}\u{43d}\u{435}\u{435} \u{43e}\u{431}\u{44a}\u{44f}\u{441}\u{43d}\u{435}\u{43d}\u{438}\u{435} \u{43d}\u{435} \u{441}\u{43e}\u{437}\u{434}\u{430}\u{451}\u{442}\u{441}\u{44f} \u{43f}\u{43e}\u{432}\u{442}\u{43e}\u{440}\u{43d}\u{43e}. \u{412}\u{44b}\u{437}\u{44b}\u{432}\u{430}\u{44e}\u{449}\u{438}\u{439} \u{43a}\u{43e}\u{434}, \u{43c}\u{430}\u{440}\u{448}\u{440}\u{443}\u{442}\u{438}\u{437}\u{430}\u{446}\u{438}\u{44f}, \u{437}\u{430}\u{432}\u{435}\u{440}\u{448}\u{435}\u{43d}\u{438}\u{435} \u{43f}\u{43e}\u{431}\u{43e}\u{447}\u{43d}\u{44b}\u{445} \u{44d}\u{444}\u{444}\u{435}\u{43a}\u{442}\u{43e}\u{432} \u{438} \u{434}\u{43e}\u{441}\u{442}\u{430}\u{432}\u{43a}\u{430} \u{43d}\u{435} \u{43f}\u{43e}\u{434}\u{442}\u{432}\u{435}\u{440}\u{436}\u{434}\u{435}\u{43d}\u{44b}.</p><h2>\u{425}\u{43e}\u{434} \u{43f}\u{440}\u{43e}\u{446}\u{435}\u{441}\u{441}\u{430}</h2>",escape(&spec.scope)));
    html.push_str(&steps(
        &spec.steps,
        &processes,
        &links,
        &mut BTreeSet::new(),
        &mut 0,
        0,
    )?);
    html.push_str("</main></body></html>");
    let parent = output
        .parent()
        .ok_or_else(|| invalid("preview output parent is unavailable"))?;
    fs::create_dir_all(parent).map_err(io_error)?;
    let stage = tempfile::tempdir_in(parent).map_err(io_error)?;
    fs::write(stage.path().join("index.html"), &html).map_err(io_error)?;
    let manifest = json!({"schema":"codeclew-linked-process-preview-result/1.0","inputDigest":hash_bytes(&raw),"outputDirectory":output,"processes":spec.processes.iter().map(|p|json!({"id":p.id,"service":p.service,"revision":p.revision,"href":links[p.id.as_str()].href,"proof":links[p.id.as_str()].proof})).collect::<Vec<_>>(),"modelCalls":0,"captures":0,"runtime":"NOT_VERIFIED","internalNarrativeGeneration":false,"indexDigest":hash_bytes(html.as_bytes())});
    fs::write(
        stage.path().join("manifest.json"),
        serde_json::to_vec_pretty(&manifest)
            .map_err(|_| invalid("preview manifest encoding failed"))?,
    )
    .map_err(io_error)?;
    fs::rename(stage.path(), output).map_err(io_error)?;
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;
    // Closed-contract conformance fixtures, not compiler or runtime evidence.
    fn fixture(root: &Path) -> (PathBuf, Value) {
        let repo = root.join("existing export");
        let native = root.join("native");
        fs::create_dir_all(repo.join("processes")).unwrap();
        fs::create_dir_all(native.join("scenarios")).unwrap();
        let page = "<html><h1>Existing explanation</h1><div id=\"request\">Existing body must not be duplicated.</div></html>";
        fs::write(repo.join("processes/existing.html"), page).unwrap();
        let report = json!({"schema":"codeclew-documentation-catalogue-export/1.0","processes":[{"route":"processes/existing.html","subject":"service:repo","operation":"repo-operation","operationDigest":"operation-digest","acceptedVersion":{"verification":"VERIFIED_WITH_LIMITATIONS","sourceRevisions":{"repo":"revision-repo"}}}]});
        let report = serde_json::to_vec(&report).unwrap();
        fs::write(repo.join("export.json"), &report).unwrap();
        let source_page = "<html><h1>Native service process</h1><div id=\"process-overview\">Retained native source</div></html>";
        fs::write(native.join("scenarios/client.html"), source_page).unwrap();
        let data = serde_json::to_vec(&json!({"subject":"scenario:client","revisions":{"manager":"revision-manager"},"processThread":{"status":"SOURCE_BOUND_DECLARATION","authority":"DECLARED_MEANING_EXACT_RETAINED_SOURCE_NOT_COMPILER_CALL_GRAPH","rootMethod":"root","methods":{"root":{"source":{"service":"manager","revision":"revision-manager"}}}}})).unwrap();
        fs::write(native.join("scenarios/client.json"), &data).unwrap();
        let publication=serde_json::to_vec(&json!({"files":{"scenarios/client.json":hash_bytes(&data),"scenarios/client.html":hash_bytes(source_page.as_bytes())}})).unwrap();
        fs::write(native.join("publication.json"), &publication).unwrap();
        let spec = json!({"schema":"codeclew-linked-process-preview/1.0","title":"Linked processes","scope":"A declared selected continuation","processes":[
            {"id":"manager","title":"Native client process","service":"manager","revision":"revision-manager","binding":{"kind":"NATIVE_SCENARIO_PAGE","directory":native,"publicationDigest":hash_bytes(&publication),"scenario":"client","scenarioDigest":hash_bytes(&data)}},
            {"id":"repo","title":"Existing repository process","service":"repo","revision":"revision-repo","binding":{"kind":"CATALOGUE_PAGE","directory":repo,"exportDigest":hash_bytes(&report),"route":"processes/existing.html","pageDigest":hash_bytes(page.as_bytes()),"operation":"repo-operation","operationDigest":"operation-digest"}}
        ],"steps":[{"id":"request","process":"manager","children":[{"id":"receiver","process":"repo","anchor":"request"}]}]});
        (root.join("input.json"), spec)
    }
    fn save(path: &Path, input: &Value) {
        fs::write(path, serde_json::to_vec(input).unwrap()).unwrap();
    }
    #[test]
    fn native_preview_links_exact_pages_and_reuses_step_container_without_internal_body() {
        let root = tempfile::tempdir().unwrap();
        let (input, spec) = fixture(root.path());
        save(&input, &spec);
        let output = root.path().join("preview");
        let result = render(&input, &output).unwrap();
        let html = fs::read_to_string(output.join("index.html")).unwrap();
        assert_eq!(html.matches("class=\"step-node\"").count(), 2);
        assert_eq!(html.matches("class=\"nested-steps\"").count(), 2);
        assert!(html.contains("existing%20export/processes/existing.html#request"));
        assert!(!html.contains("Existing body must not be duplicated"));
        assert!(!html.contains("Retained native source"));
        assert_eq!(result["modelCalls"], 0);
        assert_eq!(result["internalNarrativeGeneration"], false);
        assert_eq!(fs::read_dir(&output).unwrap().count(), 2);
        assert!(
            render(&input, &output)
                .unwrap_err()
                .message
                .contains("new output directory")
        );
    }
    #[test]
    fn changed_pages_foreign_bindings_and_missing_anchors_are_rejected_before_output() {
        for case in [
            "page",
            "export",
            "revision",
            "operation",
            "route",
            "anchor",
            "native_root",
            "duplicate",
            "body",
        ] {
            let root = tempfile::tempdir().unwrap();
            let (input, mut spec) = fixture(root.path());
            match case {
                "page" => fs::write(
                    root.path().join("existing export/processes/existing.html"),
                    "changed",
                )
                .unwrap(),
                "export" => {
                    fs::write(root.path().join("existing export/export.json"), "{}").unwrap()
                }
                "revision" => spec["processes"][1]["revision"] = json!("foreign"),
                "operation" => spec["processes"][1]["binding"]["operation"] = json!("foreign"),
                "route" => {
                    spec["processes"][1]["binding"]["route"] = json!("processes/../existing.html")
                }
                "anchor" => spec["steps"][0]["children"][0]["anchor"] = json!("absent"),
                "native_root" => spec["processes"][0]["service"] = json!("foreign"),
                "duplicate" => spec["steps"][0]["children"][0]["id"] = json!("request"),
                "body" => spec["steps"][0]["text"] = json!("Invented internal explanation"),
                _ => unreachable!(),
            }
            save(&input, &spec);
            let output = root.path().join("preview");
            assert!(render(&input, &output).is_err(), "{case}");
            assert!(!output.exists(), "{case}");
        }
    }
    #[test]
    fn symbolic_page_files_are_not_followed() {
        let root = tempfile::tempdir().unwrap();
        let (input, spec) = fixture(root.path());
        save(&input, &spec);
        let page = root.path().join("existing export/processes/existing.html");
        let original = root.path().join("original.html");
        fs::rename(&page, &original).unwrap();
        std::os::unix::fs::symlink(&original, &page).unwrap();
        assert!(render(&input, &root.path().join("preview")).is_err());
    }
}
