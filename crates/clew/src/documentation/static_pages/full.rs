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

/// Prose, threads, assessments and their retained evidence stay on the existing
/// publication contract. Selected facts are a separate reader evidence view.
pub(super) fn render(
    repo: &Repository,
    checked: &Check,
    snapshot: &str,
    publication: &str,
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
            payload["contracts"] = json!(
                service
                    .observations
                    .values()
                    .filter(|o| o.kind == "CONTRACT_OPERATION")
                    .collect::<Vec<_>>()
            );
            payload["sources"] = json!(service.sources);
            // Keep operationSources/operationContracts/operationStates and all
            // narrative/thread data byte-equivalent as parsed frozen records.
        }
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
    files.insert("catalog.html".into(), reader::catalog(&files).into_bytes());
    files.insert("history.html".into(), reader::page("Selected publication", &format!("<h1>Selected publication</h1><p><code>{}</code></p><p>Selected source snapshot: <code>{}</code></p><p><a href=\"index.html\">Documentation</a></p>",render::escape(publication),render::escape(snapshot))).into_bytes());
    let result = json!({"schema":"codeclew-native-pages-full-manifest/1.0","status":"RENDERED","snapshot":snapshot,"publication":publication,
        "authority":"FROZEN_NARRATIVES_WITH_SEPARATE_SELECTED_SNAPSHOT_EVIDENCE","inputDigest":checked.input_digest,"contextDigest":checked.context_digest,
        "frozenBindingsDigest":digest(&bindings)?,"pages":pages,"output":output,"index":"index.html","proseAuthored":false,
        "files":files.iter().map(|(path,bytes)|json!({"path":path,"digest":crate::canonical::hash_bytes(bytes)})).collect::<Vec<_>>()});
    files.insert(
        "pages-manifest.json".into(),
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
