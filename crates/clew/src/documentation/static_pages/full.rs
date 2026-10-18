//! Full immutable publication consumption through the existing service/process reader.
use crate::documentation::{
    check::Check,
    digest, history, invalid, io_error, reader, render,
    store::{self, Repository},
};
use crate::error::ClewError;
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, io::Write, path::Path};

const MAX_BYTES: u64 = 64 * 1024 * 1024;

pub(super) struct ReadingInputs<'a> {
    pub reading_plan: Option<&'a Path>,
    pub contract_comparison: Option<&'a Path>,
    pub data_model: Option<&'a Path>,
    pub format: super::PageFormat,
}

/// Prose, threads, assessments and their retained evidence stay on the existing
/// publication contract. Selected facts are a separate reader evidence view.
pub(super) fn render(
    repo: &Repository,
    checked: &Check,
    snapshot: &str,
    publication: &str,
    inputs: ReadingInputs<'_>,
    output: &Path,
) -> Result<Value, ClewError> {
    validate_output(output)?;
    let policy = crate::documentation::endpoint_publication::load(repo)?;
    let policy_digest = policy.digest()?;
    if !policy.exclusions.is_empty() {
        return Err(invalid(
            "full Pages cannot export a frozen publication under endpoint exclusions; use declaration selections or remove the exclusions explicitly",
        ));
    }
    let mut budget = history::FrozenInputBudget {
        remaining_bytes: MAX_BYTES,
    };
    let (manifest, raw) = history::read_frozen_bindings(repo, publication, &mut budget)?;
    let (_, bindings) =
        history::validate_frozen_bindings(repo, manifest.clone(), &raw, &mut BTreeMap::new())?;
    let mut files = BTreeMap::new();
    let mut remaining = MAX_BYTES;
    let mut pages = Vec::new();
    for (name, expected) in &manifest.files {
        store::relative(name)?;
        let path = repo.path(&format!("docs/generated/{publication}/{name}"))?;
        let metadata = fs::symlink_metadata(&path).map_err(io_error)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > remaining {
            return Err(invalid(
                "full pages frozen input is unsafe or exceeds its byte budget",
            ));
        }
        let bytes = fs::read(&path).map_err(io_error)?;
        if bytes.len() as u64 > remaining || crate::canonical::hash_bytes(&bytes) != *expected {
            return Err(invalid("full pages frozen input changed after validation"));
        }
        remaining -= bytes.len() as u64;
        files.insert(name.clone(), bytes);
    }
    let questions =
        super::question_reading::load(inputs.reading_plan, publication, &files, checked)?;
    let model = super::data_model::load(
        inputs.data_model,
        publication,
        snapshot,
        &files,
        checked,
        &questions,
    )?;
    let comparisons = super::contract_comparison::load(
        inputs.contract_comparison,
        publication,
        snapshot,
        checked,
    )?;
    let process_theme = questions
        .values()
        .any(|guide| guide["presentation"] == "SEQUENTIAL");
    for name in manifest.files.keys().filter(|name| {
        (name.starts_with("services/") || name.starts_with("scenarios/")) && name.ends_with(".json")
    }) {
        let mut payload: Value = serde_json::from_slice(&files[name]).map_err(io_error)?;
        let subject = payload["subject"]
            .as_str()
            .ok_or_else(|| invalid("full pages reader subject is missing"))?
            .to_owned();
        let narrative = bindings
            .narratives
            .get(&subject)
            .ok_or_else(|| invalid("full pages subject has no frozen narrative"))?;
        if payload["operations"] != json!(narrative.operations) {
            return Err(invalid(
                "full pages operation bodies differ from frozen narrative",
            ));
        }
        if let Some(guide) = questions.get(&subject) {
            payload["questionReader"] = guide.clone();
        }
        payload["selectedSnapshot"] = json!(snapshot);
        payload["frozenPublication"] = json!(publication);
        payload["selectedEvidence"] = json!({"revisions":checked.services.iter().map(|(id,e)|(id,&e.revision)).collect::<BTreeMap<_,_>>(),
            "services":checked.services.iter().map(|(id,e)|(id,json!({"coverage":e.coverage,"extractor":e.extractor,"boundaries":e.boundaries}))).collect::<BTreeMap<_,_>>(),
            "authority":"SELECTED_SNAPSHOT_NOT_RUNTIME_PROOF"});
        if let Some(service) = subject
            .strip_prefix("service:")
            .and_then(|id| checked.services.get(id))
        {
            payload["catalogue"] = json!(service.entrypoints);
            // The frozen inventory can predate route discovery. Keep it as
            // retained evidence, and project the selected inventory separately.
            let inventory = crate::documentation::sections::inventory(&service.service, checked);
            payload["selectedPublicBoundaryIds"] = json!(
                inventory["publicBoundaries"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|entry| &entry["id"])
                    .collect::<Vec<_>>()
            );
            payload["selectedBoundaryInventory"] = inventory;
            payload["entryProcesses"] =
                retained_entry_processes(&files, &service.service, &service.entrypoints)?;
            payload["contracts"] = json!(
                service
                    .observations
                    .values()
                    .filter(|o| o.kind == "CONTRACT_OPERATION")
                    .collect::<Vec<_>>()
            );
            merge_sources(&mut payload, json!(service.sources))?;
            if let Some(comparison) = comparisons
                .as_ref()
                .filter(|c| c["service"] == service.service)
            {
                payload["contractComparison"] = comparison.clone();
            }
            // Keep operationSources/operationContracts/operationStates and all
            // narrative/thread data byte-equivalent as parsed frozen records.
        }
        super::data_model::attach_process(model.as_ref(), &subject, &mut payload)?;
        let html_name = name.strip_suffix(".json").unwrap().to_owned() + ".html";
        let html = render::html(&payload)?;
        let prefix = "../";
        let navigation = reader::navigation_language(
            prefix,
            "../overview.html",
            Some("../history.html"),
            &manifest.files.keys().cloned().collect::<Vec<_>>(),
            payload["language"].as_str().unwrap_or("en"),
        );
        files.insert(
            html_name.clone(),
            reader::decorate(&html, &navigation).into_bytes(),
        );
        files.insert(
            name.clone(),
            serde_json::to_vec(&payload).map_err(io_error)?,
        );
        pages.push(json!({"subject":subject,"html":html_name,"json":name}));
    }
    // Reuse the publication's overview and all diagrams, contracts and source
    // payloads. Adapt navigation paths, never authored explanation content.
    for (name, bytes) in files.iter_mut().filter(|(name, _)| {
        name.ends_with(".html") && !name.starts_with("services/") && !name.starts_with("scenarios/")
    }) {
        let text = String::from_utf8(bytes.clone()).map_err(io_error)?;
        *bytes = text
            .replace("../../history.html", "history.html")
            .into_bytes();
        let _ = name;
    }
    if let Some(guide) = questions.get("system:overview") {
        let original = files
            .get("overview.html")
            .ok_or_else(|| invalid("system guide has no frozen overview"))?
            .clone();
        let html = super::question_reading::overview_html(
            std::str::from_utf8(&original).map_err(io_error)?,
            guide,
        )?;
        files.insert("retained-overview.html".into(), original);
        files.insert("overview.html".into(), html.into_bytes());
        files.insert(
            "system-reading-guide.json".into(),
            serde_json::to_vec_pretty(guide).map_err(io_error)?,
        );
    }
    if let Some(model) = &model {
        super::data_model::write(model, &mut files)?;
    }
    files.insert(
        "index.html".into(),
        files
            .get("overview.html")
            .ok_or_else(|| invalid("frozen full publication overview is missing"))?
            .clone(),
    );
    files.remove("root-overview.html");
    for name in ["publication.json", "bindings.json", "inputs.json"] {
        if let Some(bytes) = files.remove(name) {
            files.insert(format!("frozen-{name}"), bytes);
        }
    }
    if let Some(comparison) = &comparisons {
        files.insert(
            "http-contract-comparison.json".into(),
            serde_json::to_vec_pretty(comparison).map_err(io_error)?,
        );
    }
    let navigation = reader::navigation("", "overview.html", Some("history.html"), &[]);
    let catalog = reader::decorate(&reader::catalog(&files), &navigation);
    files.insert("catalog.html".into(), catalog.into_bytes());
    let history = reader::page(
        "Selected publication",
        &format!(
            "<h1>Selected publication</h1><p><code>{}</code></p><p>Selected source snapshot: <code>{}</code></p><p><a href=\"index.html\">Documentation</a></p>",
            render::escape(publication),
            render::escape(snapshot)
        ),
    );
    files.insert(
        "history.html".into(),
        reader::decorate(&history, &navigation).into_bytes(),
    );
    let navigation_context = reader::navigation_context(&files)?;
    for (name, bytes) in &mut files {
        if name.ends_with(".html") {
            let html = std::str::from_utf8(bytes).map_err(io_error)?;
            let language = html
                .split_once("<html lang=\"")
                .and_then(|(_, rest)| rest.split_once('"').map(|(lang, _)| lang))
                .unwrap_or("en");
            let navigation = if name.contains('/') {
                reader::navigation_language(
                    "../",
                    "../overview.html",
                    Some("../history.html"),
                    &[],
                    language,
                )
            } else {
                reader::navigation_language(
                    "",
                    "overview.html",
                    Some("history.html"),
                    &[],
                    language,
                )
            };
            *bytes = reader::attach_navigation_context(
                &reader::refresh_chrome(html, &navigation),
                &navigation_context,
            )?
            .into_bytes();
        }
    }
    if process_theme {
        for (name, bytes) in &mut files {
            if name.ends_with(".html") {
                let html = std::str::from_utf8(bytes).map_err(io_error)?;
                *bytes = reader::process_theme(html).into_bytes();
            }
        }
    }
    let mut result = json!({"schema":"codeclew-native-pages-full-manifest/1.0","status":"RENDERED","snapshot":snapshot,"publication":publication,
        "authority":"FROZEN_NARRATIVES_WITH_SEPARATE_SELECTED_SNAPSHOT_EVIDENCE","inputDigest":checked.input_digest,"contextDigest":checked.context_digest,
        "frozenBindingsDigest":digest(&bindings)?,"pages":pages,"output":output,"index":"index.html","proseAuthored":false,"questionReadingProfile":!questions.is_empty(),"dataModelReadingProfile":model.is_some(),
        "files":files.iter().map(|(path,bytes)|json!({"path":path,"digest":crate::canonical::hash_bytes(bytes)})).collect::<Vec<_>>()});
    let portable = inputs.format == super::PageFormat::Docusaurus;
    if portable {
        (files, result) = super::docusaurus::project(&files, &result, &inputs)?;
    }
    files.insert(
        if portable {
            "export-manifest.json"
        } else {
            "pages-manifest.json"
        }
        .into(),
        serde_json::to_vec_pretty(&result).map_err(io_error)?,
    );
    validate_output(output)?;
    let parent = output
        .parent()
        .filter(|p| p.is_dir())
        .ok_or_else(|| invalid("full pages output parent must exist"))?;
    let temporary = tempfile::Builder::new()
        .prefix(".pages-pending-")
        .tempdir_in(parent)
        .map_err(io_error)?;
    for (name, bytes) in &files {
        store::relative(name)?;
        let target = temporary.path().join(name);
        fs::create_dir_all(target.parent().unwrap()).map_err(io_error)?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
            .map_err(io_error)?;
        file.write_all(bytes).map_err(io_error)?;
        file.sync_all().map_err(io_error)?;
    }
    let _lock = repo.lock()?;
    if crate::documentation::endpoint_publication::load(repo)?.digest()? != policy_digest {
        return Err(invalid(
            "endpoint publication policy changed during full Pages rendering; retry into a new output directory",
        ));
    }
    validate_output(output)?;
    if output.exists() {
        fs::remove_dir(output).map_err(io_error)?;
    }
    fs::rename(temporary.path(), output).map_err(io_error)?;
    Ok(result)
}

fn retained_entry_processes(
    files: &BTreeMap<String, Vec<u8>>,
    service: &str,
    entries: &[crate::documentation::model::Entrypoint],
) -> Result<Value, ClewError> {
    let mut links: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for (path, bytes) in files
        .iter()
        .filter(|(path, _)| path.starts_with("scenarios/") && path.ends_with(".json"))
    {
        let page: Value = serde_json::from_slice(bytes).map_err(io_error)?;
        let thread = &page["processThread"];
        if thread["status"] != "SOURCE_BOUND_DECLARATION" {
            continue;
        }
        let Some(root_id) = thread["rootMethod"].as_str() else {
            continue;
        };
        let root = &thread["methods"][root_id];
        let Some(entry) = entries.iter().find(|entry| {
            root["source"]["service"] == service
                && root["entrypoint"] == entry.id
                && root["symbol"] == entry.symbol
        }) else {
            continue;
        };
        links.entry(entry.id.clone()).or_default().push(json!({
            "id":page["process"]["definition"]["id"], "title":page["title"],
            "href":format!("../{}.html#process-overview", path.strip_suffix(".json").unwrap()),
            "binding":"EXACT_RETAINED_ROOT_DECLARATION",
            "sourceRevision":root["source"]["revision"],
            "state":page["operationStates"]["process-overview"]
        }));
    }
    Ok(json!(links))
}

fn merge_sources(payload: &mut Value, selected: Value) -> Result<(), ClewError> {
    let retained = payload["sources"]
        .as_object_mut()
        .ok_or_else(|| invalid("full pages retained source records are missing"))?;
    let selected_records = selected
        .as_object()
        .ok_or_else(|| invalid("full pages selected source records are missing"))?;
    // Stable source IDs can recur at a different revision. Accepted citations
    // keep their complete original record, including cross-service evidence.
    for (id, source) in selected_records {
        retained.entry(id.clone()).or_insert_with(|| source.clone());
    }
    payload["selectedSources"] = selected;
    Ok(())
}

fn validate_output(output: &Path) -> Result<(), ClewError> {
    if let Ok(metadata) = fs::symlink_metadata(output)
        && (!metadata.is_dir()
            || metadata.file_type().is_symlink()
            || fs::read_dir(output).map_err(io_error)?.next().is_some())
    {
        return Err(invalid(
            "full pages output must be a new or empty regular directory",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_links_require_the_exact_retained_root_declaration() {
        let entry = crate::documentation::model::Entrypoint {
            id: "svc-save".into(),
            service: "svc".into(),
            symbol: "exact:save".into(),
            kind: "HTTP_ENDPOINT".into(),
            trigger: json!({}),
            source_ids: vec![],
            dependency_ids: vec![],
            boundaries: vec![],
        };
        let page = json!({"title":"Saved flow", "process":{"definition":{"id":"save"}},
            "processThread":{"status":"SOURCE_BOUND_DECLARATION", "rootMethod":"root",
                "methods":{"root":{"entrypoint":"svc-save", "symbol":"exact:save",
                    "source":{"service":"svc", "revision":"old-revision"}}}},
            "operationStates":{"process-overview":{"freshness":"STALE", "verification":"UNASSESSED"}}});
        let mut files = BTreeMap::new();
        files.insert(
            "scenarios/save.json".into(),
            serde_json::to_vec(&page).unwrap(),
        );
        for (name, field, value) in [
            ("same-name-other-service", "service", "other"),
            ("same-id-other-symbol", "symbol", "other:save"),
            ("same-symbol-other-entry", "entrypoint", "other-entry"),
        ] {
            let mut other = page.clone();
            if field == "service" {
                other["processThread"]["methods"]["root"]["source"][field] = json!(value);
            } else {
                other["processThread"]["methods"]["root"][field] = json!(value);
            }
            files.insert(
                format!("scenarios/{name}.json"),
                serde_json::to_vec(&other).unwrap(),
            );
        }
        let links = retained_entry_processes(&files, "svc", &[entry]).unwrap();
        let rows = links["svc-save"].as_array().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["href"], "../scenarios/save.html#process-overview");
        assert_eq!(rows[0]["sourceRevision"], "old-revision");
        assert_eq!(rows[0]["state"]["freshness"], "STALE");
        assert_eq!(rows[0]["binding"], "EXACT_RETAINED_ROOT_DECLARATION");
    }

    #[test]
    fn retained_citations_keep_original_revision_and_cross_service_records() {
        let original = json!({
            "same-id":{"revision":"original","url":"original-url","evidenceDigest":"original-evidence","occurrence":{"snapshot":"original"}},
            "other-service-id":{"service":"other","revision":"other-original"}
        });
        let selected = json!({
            "same-id":{"revision":"selected","url":"selected-url","evidenceDigest":"selected-evidence","occurrence":{"snapshot":"selected"}},
            "new-id":{"revision":"selected"}
        });
        let mut payload = json!({"sources":original});
        merge_sources(&mut payload, selected.clone()).unwrap();
        for (id, source) in original.as_object().unwrap() {
            assert_eq!(payload["sources"][id], *source);
        }
        assert_eq!(payload["sources"]["new-id"], selected["new-id"]);
        assert_eq!(payload["selectedSources"], selected);
    }
}
