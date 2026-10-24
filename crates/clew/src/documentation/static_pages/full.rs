//! Full immutable publication consumption through the existing service/process reader.
use crate::documentation::{
    check::Check,
    digest, history, invalid, io_error, reader, render,
    store::{self, Repository},
};
use crate::error::ClewError;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::Path,
};

const MAX_BYTES: u64 = 64 * 1024 * 1024;

pub(super) struct ReadingInputs<'a> {
    pub reading_plan: Option<&'a Path>,
    pub contract_comparison: Option<&'a Path>,
    pub data_model: Option<&'a Path>,
    pub format: super::PageFormat,
    pub language: Option<&'a str>,
    pub source_policy: &'a crate::documentation::source_export::Policy,
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
    let mut source_projection =
        crate::documentation::source_export::Projection::new(inputs.source_policy, checked)?;
    let policy = crate::documentation::endpoint_publication::load(repo)?;
    let policy_digest = policy.digest()?;
    if !policy.exclusions.is_empty() {
        return Err(invalid(
            "full Pages cannot export a frozen publication under endpoint exclusions; use declaration selections or remove the exclusions explicitly",
        ));
    }
    let (manifest, original_manifest, raw, mut accounting) = if inputs.source_policy.remote() {
        let admitted =
            super::frozen_export::read(repo, publication, &inputs.source_policy.subjects)?;
        (
            admitted.selected,
            admitted.original,
            admitted.bindings,
            Some(admitted.accounting),
        )
    } else {
        let mut budget = history::FrozenInputBudget {
            remaining_bytes: MAX_BYTES,
        };
        let (manifest, raw) = history::read_frozen_bindings(repo, publication, &mut budget)?;
        (manifest.clone(), manifest, raw, None)
    };
    let bindings = if inputs.source_policy.remote() {
        let (bindings, stats) = history::validate_frozen_reading_bindings(
            repo,
            &original_manifest,
            &raw,
            &mut BTreeMap::new(),
        )?;
        if let Some(accounting) = accounting.as_mut() {
            accounting["semanticValidation"] = stats;
        }
        bindings
    } else {
        history::validate_frozen_bindings(
            repo,
            original_manifest.clone(),
            &raw,
            &mut BTreeMap::new(),
        )?
        .1
    };
    let requested_language = inputs
        .language
        .or(bindings.documentation_language.as_deref());
    let language = requested_language.unwrap_or("en");
    crate::documentation::language::validate(Some(language))?;
    let mut files = BTreeMap::new();
    let mut remaining = MAX_BYTES;
    let mut pages = Vec::new();
    let mut groups = BTreeMap::<String, Vec<String>>::new();
    for name in manifest.files.keys() {
        let owner = if inputs.source_policy.remote() {
            super::frozen_export::subject(name).unwrap_or_else(|| "__shared".into())
        } else {
            "__embedded".into()
        };
        groups.entry(owner).or_default().push(name.clone());
    }
    let mut retained_bytes = 0u64;
    for (owner, names) in groups {
        let mut group = BTreeMap::new();
        for name in names {
            store::relative(&name)?;
            let path = repo.path(&format!("docs/generated/{publication}/{name}"))?;
            let metadata = fs::symlink_metadata(&path).map_err(io_error)?;
            let limit = if inputs.source_policy.remote() {
                MAX_BYTES
            } else {
                remaining
            };
            if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > limit {
                return Err(invalid(
                    "full pages frozen record is unsafe or exceeds its byte budget",
                ));
            }
            let bytes = fs::read(&path).map_err(io_error)?;
            if bytes.len() as u64 > limit
                || crate::canonical::hash_bytes(&bytes) != manifest.files[&name]
            {
                return Err(invalid("full pages frozen input changed after validation"));
            }
            if !inputs.source_policy.remote() {
                remaining -= bytes.len() as u64;
            }
            group.insert(name, bytes);
        }
        if inputs.source_policy.remote() {
            source_projection.register_frozen_files(&group)?;
            if owner != "__shared" {
                // Project one service/process HTML/Markdown group before retaining
                // it. Original JSON remains available for narrative comparison and
                // exact generated-appendix replacement in the final projection.
                let originals: Vec<_> = group
                    .iter()
                    .filter(|(name, _)| name.ends_with(".json"))
                    .map(|(name, bytes)| (name.clone(), bytes.clone()))
                    .collect();
                source_projection.files(&mut group)?;
                group.extend(originals);
            }
        }
        retained_bytes =
            retained_bytes.saturating_add(group.values().map(|b| b.len() as u64).sum::<u64>());
        if inputs.source_policy.remote()
            && retained_bytes > crate::documentation::check::PORTABLE_CACHE_MAX_BYTES
        {
            return Err(crate::error::ClewError::new(
                crate::error::ErrorCode::SliceBudgetExceeded,
                "remote reading inputs exceed 128 MiB retained buffer; select fewer export-subjects",
            ));
        }
        files.extend(group);
    }
    let original_bindings = raw;
    // Publication metadata is outside its own file inventory. Read the exact
    // admitted file separately; including its own hash would be recursive.
    let publication_path = repo.path(&format!("docs/generated/{publication}/publication.json"))?;
    let mut publication_bytes = Vec::new();
    fs::File::open(&publication_path)
        .map_err(io_error)?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut publication_bytes)
        .map_err(io_error)?;
    if publication_bytes.len() as u64 > MAX_BYTES
        || digest(
            &serde_json::from_slice::<history::Publication>(&publication_bytes)
                .map_err(io_error)?,
        )? != digest(&original_manifest)?
    {
        return Err(invalid("publication metadata changed after validation"));
    }
    let original_publication_digest = crate::canonical::hash_bytes(&publication_bytes);
    files.insert("publication.json".into(), publication_bytes);
    if inputs.source_policy.remote() {
        // Projection must see selected logical source records, not opaque text
        // chunks (which can also contain unused records from an older packing).
        source_projection.register_frozen_files(&files)?;
        let mut logical =
            crate::documentation::bindings_storage::resolve(repo, &original_bindings)?.0;
        source_projection.register_sources(&logical)?;
        source_projection.value(&mut logical)?;
        files.insert(
            "bindings.json".into(),
            crate::documentation::bytes(&logical)?,
        );
    }
    source_projection.register_frozen_files(&files)?;
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
    let historical = inputs.format == super::PageFormat::Historical;
    let process_theme = !historical
        && questions
            .values()
            .any(|guide| guide["presentation"] == "SEQUENTIAL");
    let mut guide_gaps = BTreeMap::new();
    let mut guide_notice_files = BTreeMap::new();
    let mut authored_gaps = BTreeMap::new();
    let mut authored_notice_files = BTreeMap::new();
    let mut narrative_gaps = BTreeMap::new();
    let mut narrative_notice_files = BTreeMap::new();
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
            let original_path = format!(
                "{}.reading-guide.original.json",
                name.trim_end_matches(".json")
            );
            files.insert(
                original_path.clone(),
                serde_json::to_vec_pretty(guide).map_err(io_error)?,
            );
            if let Some(gap) =
                super::question_reading::translation_gap(guide, requested_language, &original_path)?
            {
                payload["questionReaderTranslationGap"] = gap.clone();
                guide_gaps.insert(subject.clone(), gap);
            }
        }
        payload["language"] = json!(language);
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
        let gaps = super::prose_language::attach_payload(
            &mut payload,
            name,
            requested_language,
            &mut files,
        )?;
        let narrative = super::prose_language::attach_narrative(
            &mut payload,
            name,
            requested_language,
            &mut files,
        )?;
        let html_name = name.strip_suffix(".json").unwrap().to_owned() + ".html";
        if !narrative.is_empty() {
            narrative_gaps.insert(subject.clone(), narrative.clone());
            narrative_notice_files.insert(html_name.clone(), narrative);
        }
        if !gaps.is_empty() {
            authored_gaps.insert(subject.clone(), gaps.clone());
            authored_notice_files.insert(html_name.clone(), gaps);
        }
        if payload["questionReaderTranslationGap"].is_object() {
            guide_notice_files.insert(
                html_name.clone(),
                payload["questionReaderTranslationGap"].clone(),
            );
        }
        if inputs.source_policy.remote() {
            source_projection.register_sources(&payload)?;
            source_projection.value(&mut payload)?;
        }
        if historical {
            // Keep the original operation catalogue, tabs and service navigation.
            // Optional guides remain exact reference files, never a UI replacement.
            if let Some(object) = payload.as_object_mut() {
                object.remove("questionReader");
                object.remove("processDataFlow");
            }
        }
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
        let original_path = "system-reading-guide.original.json";
        files.insert(
            original_path.into(),
            serde_json::to_vec_pretty(guide).map_err(io_error)?,
        );
        if let Some(gap) =
            super::question_reading::translation_gap(guide, requested_language, original_path)?
        {
            guide_notice_files.insert("overview.html".into(), gap.clone());
            guide_notice_files.insert("index.html".into(), gap.clone());
            guide_gaps.insert("system:overview".into(), gap);
        }
        if !historical {
            let html = super::question_reading::overview_html_in_language(
                std::str::from_utf8(&original).map_err(io_error)?,
                guide,
                language,
            )?;
            files.insert("retained-overview.html".into(), original);
            files.insert("overview.html".into(), html.into_bytes());
            files.insert(
                "system-reading-guide.json".into(),
                serde_json::to_vec_pretty(guide).map_err(io_error)?,
            );
        }
    }
    let model = if inputs.source_policy.remote() {
        model
            .as_ref()
            .map(|model| source_projection.typed(model))
            .transpose()?
    } else {
        model
    };
    let availability = if let Some(model) = &model {
        super::data_model::write_language(model, &mut files, requested_language)?
    } else {
        super::data_model::LanguageAvailability::default()
    };
    let model_gaps = availability.models;
    let contract_gaps = availability.contracts;
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
    let navigation =
        reader::navigation_language("", "overview.html", Some("history.html"), &[], language);
    let catalog = reader::decorate(&reader::catalog_language(&files, language), &navigation);
    files.insert("catalog.html".into(), catalog.into_bytes());
    let history = reader::page_language(
        reader::text(language, "Selected publication", "Выбранная публикация"),
        &format!(
            "<h1>{}</h1><p><code>{}</code></p><p>{}: <code>{}</code></p><p><a href=\"index.html\">{}</a></p>",
            reader::text(language, "Selected publication", "Выбранная публикация"),
            render::escape(publication),
            reader::text(
                language,
                "Selected source snapshot",
                "Выбранный снимок исходного кода"
            ),
            render::escape(snapshot),
            reader::text(language, "Documentation", "Документация")
        ),
        language,
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
            let mut navigation = if name.contains('/') {
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
            if let Some(gap) = guide_notice_files.get(name) {
                navigation.push_str(&super::question_reading::translation_gap_html(gap));
            }
            if let Some(gaps) = authored_notice_files.get(name) {
                navigation.push_str(&super::prose_language::page_notices(gaps));
            }
            if let Some(gaps) = narrative_notice_files.get(name) {
                navigation.push_str(&super::prose_language::narrative_notice_html(
                    gaps, language,
                ));
            }
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
    source_projection.files(&mut files)?;
    let projected = if inputs.source_policy.remote() {
        Some(
            serde_json::from_slice(
                files
                    .get("frozen-bindings.json")
                    .ok_or_else(|| invalid("export bindings missing"))?,
            )
            .map_err(io_error)?,
        )
    } else {
        None
    };
    files.remove("frozen-bindings.json");
    files.extend(crate::documentation::bindings_storage::export_files(
        repo,
        &original_bindings,
        projected,
    )?);
    let original_publication: Value = serde_json::from_slice(
        &files
            .remove("frozen-publication.json")
            .ok_or_else(|| invalid("original publication metadata missing"))?,
    )
    .map_err(io_error)?;
    let mut result = json!({"schema":"codeclew-native-pages-full-manifest/1.0","status":if guide_gaps.is_empty() && model_gaps.is_empty() && contract_gaps.is_empty() && authored_gaps.is_empty() && narrative_gaps.is_empty(){"RENDERED"}else{"PARTIAL"},"documentationLanguage":language,"requestedDocumentationLanguage":requested_language,"narrativeTranslationGaps":narrative_gaps,"narrativeTranslationComplete":requested_language.map(|_|narrative_gaps.is_empty()),"narrativeTranslationCoverage":["OPERATIONS","SECTIONS","NOTE_ASSESSMENTS","NOTE_ORIGINALS"],"authoredTranslationGaps":authored_gaps,"authoredTranslationComplete":requested_language.map(|_|authored_gaps.is_empty()),"declaredContractTranslationGaps":contract_gaps,"declaredContractTranslationComplete":requested_language.map(|_|contract_gaps.is_empty()),"modelTranslationGaps":model_gaps,"modelTranslationComplete":requested_language.map(|_|model_gaps.is_empty()),"readingGuideTranslationGaps":guide_gaps,"readingGuideTranslationComplete":requested_language.map(|_|guide_gaps.is_empty()),"snapshot":snapshot,"publication":publication,
        "authority":"FROZEN_NARRATIVES_WITH_SEPARATE_SELECTED_SNAPSHOT_EVIDENCE","inputDigest":checked.input_digest,"contextDigest":checked.context_digest,
        "frozenBindingsDigest":digest(&bindings)?,"pages":pages,"output":output,"index":"index.html","proseAuthored":false,"questionReadingProfile":!questions.is_empty(),"dataModelReadingProfile":model.is_some(),
        "files":files.iter().map(|(path,bytes)|json!({"path":path,"digest":crate::canonical::hash_bytes(bytes)})).collect::<Vec<_>>()});
    result["sourceExport"] = inputs.source_policy.manifest()?;
    result["exportIdentity"] = digest(&(
        publication,
        snapshot,
        inputs.source_policy,
        &result["files"],
    ))?
    .into();
    if let Some(accounting) = &accounting {
        result["exportSelection"] = accounting.clone();
        if !inputs.source_policy.subjects.is_empty() {
            for (name, raw) in &mut files {
                if !name.ends_with(".html") {
                    continue;
                }
                let html = std::str::from_utf8(raw).map_err(io_error)?;
                *raw = super::frozen_export::annotate(
                    html,
                    name,
                    &inputs.source_policy.subjects,
                    &accounting["omittedSubjects"],
                )
                .into_bytes();
            }
        }
    }
    let portable = inputs.format == super::PageFormat::Docusaurus;
    if portable {
        (files, result) = super::docusaurus::project(&files, &result, &inputs)?;
    }
    if let Some(accounting) = &accounting {
        result["exportSelection"] = accounting.clone();
    }
    // The portable closure was projected before hashing. Its source-text leaf
    // units are immutable export data, not an independent projection input.
    let frozen_keys: Vec<_> = files
        .keys()
        .filter(|name| crate::documentation::bindings_storage::export_file(name))
        .cloned()
        .collect();
    let frozen: BTreeMap<_, _> = frozen_keys
        .into_iter()
        .map(|name| {
            let data = files.remove(&name).unwrap();
            (name, data)
        })
        .collect();
    source_projection.files(&mut files)?;
    files.extend(frozen);
    crate::documentation::bindings_storage::resolve_export(
        &files,
        if portable {
            "static/clew-bindings/"
        } else {
            ""
        },
    )?;
    // Original hashes describe the source publication. Export hashes describe
    // the actual reading files; neither inventory includes its own metadata.
    let prefix = if portable {
        "static/clew-bindings/"
    } else {
        ""
    };
    let binding_manifest_path = format!("{prefix}frozen-bindings.json");
    let binding_manifest: Value =
        serde_json::from_slice(&files[&binding_manifest_path]).map_err(io_error)?;
    let reading_publication = json!({"schema":"codeclew-reading-publication/1.0","originalPublication":original_publication,"originalPublicationDigest":original_publication_digest,
        "sourceExport":inputs.source_policy.manifest()?,"bindingsManifest":binding_manifest_path,
        "files":files.iter().map(|(path,raw)|(path.clone(),crate::canonical::hash_bytes(raw))).collect::<BTreeMap<_,_>>()});
    files.insert(
        format!("{prefix}frozen-publication.json"),
        crate::documentation::bytes(&reading_publication)?,
    );
    result["originalFrozenBindingsDigest"] = result["frozenBindingsDigest"].clone();
    result["originalBindingsRecordDigest"] = json!(original_manifest.files.get("bindings.json"));
    if !inputs.source_policy.subjects.is_empty() {
        result["selectedFrozenBindingsDigest"] = result["frozenBindingsDigest"].clone();
        // Selection does not validate the omitted logical records. Preserve the
        // exact original root-record pin without claiming an exhaustive digest.
        result["originalFrozenBindingsDigest"] = Value::Null;
    }
    result["bindingsClosure"] = json!({"manifest":binding_manifest_path,"digest":crate::canonical::hash_bytes(&files[&binding_manifest_path]),
        "schema":binding_manifest["schema"],"objectRoot":format!("{prefix}{}",crate::documentation::bindings_storage::OBJECT_ROOT),
        "copyRequirement":"Copy the manifest and every object in its declared closure; source publication and authoring backups remain separate."});
    result["files"] = json!(
        files
            .iter()
            .map(|(path, bytes)| json!({"path":path,"digest":crate::canonical::hash_bytes(bytes)}))
            .collect::<Vec<_>>()
    );
    result["sourceExport"] = inputs.source_policy.manifest()?;
    result["exportIdentity"] = digest(&(
        publication,
        snapshot,
        inputs.source_policy,
        &result["files"],
    ))?
    .into();
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

pub(super) fn validate_output(output: &Path) -> Result<(), ClewError> {
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
