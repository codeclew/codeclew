//! SOURCE evidence retention and comparison without a publication baseline.
//! Immutable check admission, dependency closure and object storage are shared
//! with documentation; native references add byte tracking, never semantics.
use super::{
    analysis, bindings, cache,
    check::Check,
    cli::{self, ContextArgs, ContextFormat},
    digest, invalid, io_error,
    model::Source,
    store::{self, Repository},
};
use crate::{
    error::ClewError,
    repository_snapshot::{read_git_blob_metadata, read_git_blobs},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
const SCHEMA: &str = "codeclew-prepared-source-evidence/1.0";
const NATIVE_TEXT: &str = "codeclew-registered-native-bytes/1.0";
const MAX_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum Verdict {
    Unchanged,
    Changed,
    Unknown,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NativeScope {
    service: String,
    #[serde(default)]
    files: Vec<String>,
    #[serde(default)]
    membership: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NativeFile {
    service: String,
    path: String,
    revision: String,
    mode: String,
    blob: String,
    status: String,
    content: Option<cache::ObjectRef>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NativeState {
    files: BTreeMap<String, NativeFile>,
    membership: BTreeMap<String, Value>,
    limits: Vec<Value>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Evidence {
    schema: String,
    snapshot: String,
    service: String,
    symbol: String,
    analysis_config: Value,
    dependencies: BTreeMap<String, String>,
    source_ids: Vec<String>,
    native_scopes: Vec<NativeScope>,
    native: NativeState,
}
fn args(repo: &Repository, snapshot: &str, service: &str, symbol: &str) -> ContextArgs {
    ContextArgs {
        root: repo.root.clone(),
        service: Some(service.into()),
        scenario: None,
        entrypoint: None,
        symbols: vec![symbol.into()],
        source_ids: vec![],
        dependency_ids: vec![],
        format: ContextFormat::Compact,
        refresh: false,
        snapshot: Some(snapshot.into()),
        cursor: None,
        limit: 40,
    }
}
fn config(checked: &Check, service: &str) -> Result<Value, ClewError> {
    let inputs = checked
        .source_inputs
        .as_ref()
        .ok_or_else(|| invalid("SOURCE evidence requires an admitted source-input contract"))?;
    let service = inputs
        .inputs
        .services
        .get(service)
        .ok_or_else(|| invalid("SOURCE evidence configuration is absent"))?;
    let mut value = serde_json::to_value(service).map_err(io_error)?;
    // Revision and link presentation are reported separately from analysis inputs.
    for key in ["targetRef", "sourceLinkTemplate", "title"] {
        value.as_object_mut().unwrap().remove(key);
    }
    Ok(value)
}
fn frame(
    repo: &Repository,
    checked: &Check,
    snapshot: &str,
    service: &str,
    symbol: &str,
    scopes: Vec<NativeScope>,
    native: NativeState,
) -> Result<Evidence, ClewError> {
    let items = cli::context_items(checked, &args(repo, snapshot, service, symbol), None)?;
    let initial = items
        .iter()
        .filter_map(|v| v["id"].as_str())
        .filter(|id| checked.dependencies.contains_key(*id))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if initial.is_empty() {
        return Err(invalid(
            "SOURCE evidence selector has no dependency observations",
        ));
    }
    let ids = bindings::expand_dependencies(&initial, checked)?;
    let source_ids = items
        .iter()
        .filter(|v| v["kind"] == "SOURCE")
        .filter_map(|v| v["id"].as_str().map(str::to_owned))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if source_ids.is_empty() {
        return Err(invalid(
            "SOURCE evidence selector has no exact retained source",
        ));
    }
    Ok(Evidence {
        schema: SCHEMA.into(),
        snapshot: snapshot.into(),
        service: service.into(),
        symbol: symbol.into(),
        analysis_config: config(checked, service)?,
        dependencies: ids
            .iter()
            .map(|id| (id.clone(), checked.dependencies[id].digest.clone()))
            .collect(),
        source_ids,
        native_scopes: scopes,
        native,
    })
}
fn validate_scopes(scopes: &[NativeScope]) -> Result<(), ClewError> {
    if scopes.len() > 8 {
        return Err(invalid("register at most eight native input scopes"));
    }
    let mut ids = BTreeSet::new();
    for scope in scopes {
        if !store::valid_id(&scope.service)
            || scope.files.len() > 32
            || scope.membership.len() > 16
            || scope.files.is_empty() && scope.membership.is_empty()
        {
            return Err(invalid("native input scope is empty or exceeds bounds"));
        }
        for (kind, paths) in [("file", &scope.files), ("membership", &scope.membership)] {
            for path in paths {
                store::relative(path)?;
                if path.len() > 1024
                    || path.contains(['*', '?', '[', ']', ':', '\n', '\r'])
                    || !ids.insert((scope.service.clone(), kind, path.clone()))
                {
                    return Err(invalid(
                        "native input paths must be unique literal repository paths",
                    ));
                }
            }
        }
    }
    Ok(())
}
fn capture_native(
    repo: &Repository,
    checked: &Check,
    scopes: &[NativeScope],
) -> Result<NativeState, ClewError> {
    validate_scopes(scopes)?;
    let mut state = NativeState::default();
    let mut total = 0usize;
    for scope in scopes {
        let evidence = checked.services.get(&scope.service).ok_or_else(|| {
            invalid("native inputs require a captured repository revision binding")
        })?;
        let config = checked
            .source_inputs
            .as_ref()
            .and_then(|s| s.inputs.services.get(&scope.service))
            .ok_or_else(|| invalid("native repository configuration is missing"))?;
        let source = analysis::bound_repository(repo, config)?;
        let mut command = vec![
            "ls-tree",
            "-r",
            "-z",
            "--full-tree",
            &evidence.revision,
            "--",
        ];
        command.extend(
            scope
                .files
                .iter()
                .chain(&scope.membership)
                .map(String::as_str),
        );
        let listing = analysis::git(&source, &command)?;
        let mut tree = BTreeMap::new();
        for row in listing.split('\0').filter(|r| !r.is_empty()) {
            let (header, path) = row
                .split_once('\t')
                .ok_or_else(|| invalid("native Git tree record is invalid"))?;
            let fields = header.split_whitespace().collect::<Vec<_>>();
            if fields.len() != 3
                || tree
                    .insert(
                        path.to_owned(),
                        (
                            fields[0].to_owned(),
                            fields[1].to_owned(),
                            fields[2].to_owned(),
                        ),
                    )
                    .is_some()
            {
                return Err(invalid("native Git tree identity is invalid"));
            }
        }
        if tree.len() > 512 {
            return Err(invalid(
                "native membership exceeds 512 paths; narrow the registered scope",
            ));
        }
        for prefix in &scope.membership {
            let names = tree
                .keys()
                .filter(|p| *p == prefix || p.starts_with(&format!("{prefix}/")))
                .cloned()
                .collect::<Vec<_>>();
            let key = format!("{}:{prefix}", scope.service);
            if names.is_empty() {
                state
                    .limits
                    .push(json!({"reason":"NATIVE_MEMBERSHIP_EMPTY_OR_MISSING","input":key}));
            }
            state.membership.insert(key, json!(names));
        }
        let blobs = scope
            .files
            .iter()
            .filter_map(|p| tree.get(p))
            .filter(|(mode, kind, _)| {
                kind == "blob" && matches!(mode.as_str(), "100644" | "100755")
            })
            .map(|(_, _, blob)| blob.as_str())
            .collect::<BTreeSet<_>>();
        let metadata = read_git_blob_metadata(&source, blobs.iter().copied())?;
        let eligible = blobs
            .iter()
            .filter(|id| {
                metadata
                    .get(**id)
                    .is_some_and(|(kind, size)| kind == "blob" && *size <= 256 * 1024)
            })
            .copied()
            .collect::<Vec<_>>();
        let contents = read_git_blobs(&source, eligible.iter().copied())?;
        for path in &scope.files {
            let key = format!("{}:{path}", scope.service);
            let (mode, kind, blob) = tree.get(path).cloned().unwrap_or_default();
            let bytes = contents.get(&blob);
            let status = if kind != "blob" || !matches!(mode.as_str(), "100644" | "100755") {
                "MISSING_OR_NONREGULAR"
            } else if bytes.is_none() {
                "OVER_BYTE_BOUND"
            } else {
                "AVAILABLE"
            };
            let content = if let Some(bytes) = bytes {
                total += bytes.len();
                if total > MAX_BYTES as usize {
                    return Err(invalid("registered native bytes exceed 2 MiB"));
                }
                if std::str::from_utf8(bytes).is_err() {
                    state.limits.push(
                        json!({"reason":"NATIVE_BYTES_NON_UTF8_TEXT_NOT_DELIVERED","input":key}),
                    );
                }
                Some(cache::put(repo, NATIVE_TEXT, bytes)?)
            } else {
                state
                    .limits
                    .push(json!({"reason":"NATIVE_INPUT_UNAVAILABLE","input":key,"status":status}));
                None
            };
            state.files.insert(
                key,
                NativeFile {
                    service: scope.service.clone(),
                    path: path.clone(),
                    revision: evidence.revision.clone(),
                    mode,
                    blob,
                    status: status.into(),
                    content,
                },
            );
        }
    }
    Ok(state)
}
fn native_items(
    repo: &Repository,
    state: &NativeState,
    checked: &Check,
) -> Result<Vec<Value>, ClewError> {
    let mut items = Vec::new();
    for (key, file) in &state.files {
        if let Some(reference) = &file.content {
            if reference.schema != NATIVE_TEXT {
                return Err(invalid("native byte reference schema differs"));
            }
            let bytes = cache::get(repo, reference, 256 * 1024)?
                .ok_or_else(|| invalid("retained registered native bytes are unavailable"))?;
            let Ok(text) = String::from_utf8(bytes) else {
                continue;
            };
            let id = format!("native-{}", &digest(key)?[7..27]);
            let config = checked
                .source_inputs
                .as_ref()
                .and_then(|s| s.inputs.services.get(&file.service))
                .ok_or_else(|| invalid("registered native binding service is unavailable"))?;
            let url = analysis::source_link(
                config,
                &file.revision,
                &file.path,
                1,
                text.lines().count().max(1) as u64,
            );
            items.push(json!({"kind":"SOURCE_NATIVE","id":id,"record":{"id":id,"service":file.service,"bindingService":file.service,"url":url,"file":file.path,"revision":file.revision,"startLine":1,"endLine":text.lines().count().max(1),"text":text,"textDigest":reference.digest,"authority":"REGISTERED_NATIVE_BYTES_ONLY_NO_SEMANTIC_ANALYSIS","occurrence":{"snapshot":file.revision,"blob":file.blob,"startByte":0,"endByte":reference.size},"membershipCoverage":"EXPLICIT_REGISTERED_FILES_ONLY"}}));
        }
    }
    Ok(items)
}
fn ticket(reference: &cache::ObjectRef, evidence: &Evidence) -> Value {
    json!({"schema":SCHEMA,"reference":reference,"snapshot":evidence.snapshot,"service":evidence.service,"symbol":evidence.symbol,"nativeFiles":evidence.native.files.len(),"nativeMembershipScopes":evidence.native.membership.len(),"scope":"Registered captured dependencies and explicit native inputs; not complete runtime behavior"})
}
fn store_evidence(repo: &Repository, evidence: &Evidence) -> Result<cache::ObjectRef, ClewError> {
    let bytes = super::bytes(evidence)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(invalid(
            "SOURCE evidence set exceeds 2 MiB; narrow the selector or native registration",
        ));
    }
    cache::put(repo, SCHEMA, &bytes)
}
fn packet(
    repo: &Repository,
    checked: &Check,
    evidence: &Evidence,
    reference: &cache::ObjectRef,
    include_context: bool,
) -> Result<Value, ClewError> {
    let mut result = if include_context {
        cli::context_from(
            checked,
            &args(
                repo,
                &evidence.snapshot,
                &evidence.service,
                &evidence.symbol,
            ),
            None,
            "PINNED_SNAPSHOT_NOT_REVERIFIED",
        )?
    } else {
        json!({"schema":"codeclew-docs-page/1.0","sourceAuthorities":checked.source_authorities(),"unresolved":checked.unresolved,"snapshot":evidence.snapshot,"inputDigest":checked.input_digest,"contextDigest":checked.context_digest,"authority":"PINNED_SNAPSHOT_NOT_REVERIFIED","items":[],"omitted":[],"nextCursor":null})
    };
    let native = native_items(repo, &evidence.native, checked)?;
    if include_context {
        result["items"].as_array_mut().unwrap().extend(native);
    }
    result["evidenceSet"] = ticket(reference, evidence);
    result["evidenceValidation"] = json!({"integrity":"VERIFIED_LOADED_IMMUTABLE_CHECK_CLOSURE_AND_REGISTERED_NATIVE_OBJECTS","binding":"EXACT_PINNED_REVISION_AND_SOURCE_CONFIGURATION","liveCheckoutRechecked":false,"sourceRepresentation":"EXACT_RETAINED_SOURCE_BYTES; SOURCE_ALIAS_AND_LOCAL_EXCERPTS_ARE_SEPARATE_PROJECTIONS","nativeAuthority":"REGISTERED_BYTES_AND_EXPLICIT_MEMBERSHIP_ONLY","semanticAuthority":"UNCHANGED_FROM_CAPTURE","behaviorEquivalence":"NOT_ESTABLISHED"});
    result["registeredInputLimits"] = json!({"native":evidence.native.limits,"membership":"Names only within explicit prefixes; only registered files have byte tracking","unregisteredReferences":"UNKNOWN_OUT_OF_SCOPE","coverage":checked.services.get(&evidence.service).map(|s|json!({"authority":s.extractor,"coverage":s.coverage,"boundaries":s.boundaries})),"unresolved":checked.unresolved});
    Ok(result)
}
pub(super) fn retain(
    repo: &Repository,
    snapshot: Option<&str>,
    refresh: bool,
    service: &str,
    symbol: &str,
    native_input: Option<&Path>,
) -> Result<Value, ClewError> {
    let (checked, snapshot) = if refresh {
        super::check::run_and_save_selected(repo, &BTreeSet::from([service.into()]), None)?
    } else {
        let snapshot = snapshot.ok_or_else(|| {
            invalid("SOURCE evidence requires an exact snapshot or explicit refresh")
        })?;
        (Check::load_snapshot(repo, snapshot)?, snapshot.into())
    };
    let scopes: Vec<NativeScope> = native_input
        .map(|p| store::read(p, 64 * 1024))
        .transpose()?
        .unwrap_or_default();
    let native = capture_native(repo, &checked, &scopes)?;
    let evidence = frame(repo, &checked, &snapshot, service, symbol, scopes, native)?;
    let reference = store_evidence(repo, &evidence)?;
    packet(repo, &checked, &evidence, &reference, true)
}
fn source_identity(source: &Source) -> Value {
    let mut value = serde_json::to_value(source).expect("source serialization");
    value.as_object_mut().unwrap().remove("text");
    value
}
fn maintenance_binding(checked: &Check, evidence: &Evidence) -> Result<Value, ClewError> {
    Ok(
        json!({"snapshot":evidence.snapshot,"service":evidence.service,"symbol":evidence.symbol,
        "inputDigest":checked.input_digest,"contextDigest":checked.context_digest,
        "revision":checked.services.get(&evidence.service).map(|s|&s.revision),
        "analysisConfigurationDigest":digest(&evidence.analysis_config)?,
        "registeredNativeRevisions":evidence.native.files.values().map(|f|(&f.service,&f.revision)).collect::<BTreeMap<_,_>>() }),
    )
}
#[cfg(test)]
fn file_excerpt<'a>(checked: &'a Check, service: &str, file: &str) -> Option<&'a Source> {
    checked
        .services
        .values()
        .flat_map(|s| s.sources.values())
        .filter(|s| s.service == service && s.file == file)
        .max_by_key(|s| s.text.len())
}
fn changed_window(before_source: &Source, after_source: &Source) -> Value {
    text_window(
        &before_source.text,
        &after_source.text,
        source_identity(before_source),
        source_identity(after_source),
    )
}
fn text_window(before: &str, after: &str, before_citation: Value, after_citation: Value) -> Value {
    let prefix = before
        .chars()
        .zip(after.chars())
        .take_while(|(a, b)| a == b)
        .count();
    let offset = prefix.saturating_sub(80);
    json!({"representation":"CHARACTER_WINDOW_PROJECTION_NOT_EXACT_FULL_SOURCE","offsetCharacters":offset,"beforeCitation":before_citation,"afterCitation":after_citation,"before":before.chars().skip(offset).take(320).collect::<String>(),"after":after.chars().skip(offset).take(320).collect::<String>()})
}
fn scope_changes(id: &str, baseline: &Check, target: &Check) -> Vec<Value> {
    let (Some(a), Some(b)) = (baseline.dependencies.get(id), target.dependencies.get(id)) else {
        return vec![];
    };
    if a.kind != "SOURCE_SCOPE" || b.kind != "SOURCE_SCOPE" {
        return vec![];
    }
    let a_service = a.service.clone();
    let (Some(a), Some(b)) = (
        a.normalized["inventory"].as_object(),
        b.normalized["inventory"].as_object(),
    ) else {
        return vec![];
    };
    let paths = a.keys().chain(b.keys()).collect::<BTreeSet<_>>();
    paths.into_iter().filter(|p|a.get(*p)!=b.get(*p)).map(|path|{
        let mut change=json!({"reason":"REGISTERED_SOURCE_SCOPE_FILE_CHANGED","dependency":id,"file":path,"before":a.get(path),"after":b.get(path),"meaningImpact":"NOT_INFERRED"});
        let pair = baseline.services.values().flat_map(|s|s.sources.values())
            .filter(|s|s.service==a_service && s.file==*path)
            .filter_map(|before|target.services.get(&a_service)?.sources.get(&before.id).map(|after|(before,after)))
            .filter(|(before,after)|before.text!=after.text)
            .max_by_key(|(before,_)|before.text.len());
        if let Some((before,after))=pair {change["changedWindow"]=changed_window(before,after);}
        change["sourceReferences"]=json!({"before":baseline.services.get(&a_service).map(|s|s.sources.values().filter(|v|v.file==*path).map(|v|&v.id).take(8).collect::<Vec<_>>()),"after":target.services.get(&a_service).map(|s|s.sources.values().filter(|v|v.file==*path).map(|v|&v.id).take(8).collect::<Vec<_>>())});
        change
    }).collect()
}
fn native_identity(file: &NativeFile) -> Value {
    json!({"status":file.status,"mode":file.mode,"blob":file.blob,"content":file.content})
}
fn compare_frames(
    old: &Evidence,
    new: &Evidence,
    baseline: &Check,
    target: &Check,
) -> (Verdict, Vec<Value>) {
    let mut changes = Vec::new();
    let mut missing = false;
    if old.analysis_config != new.analysis_config {
        changes.push(json!({"reason":"ANALYSIS_CONFIGURATION_CHANGED","before":old.analysis_config,"after":new.analysis_config}));
    }
    for mut change in bindings::dependency_changes(&old.dependencies, target) {
        missing |= change["reason"] == "DEPENDENCY_UNAVAILABLE";
        change["reason"] = json!(if change["reason"] == "DEPENDENCY_UNAVAILABLE" {
            "DEPENDENCY_UNAVAILABLE"
        } else {
            "RECORDED_DEPENDENCY_CHANGED"
        });
        if let Some(id) = change["dependency"].as_str().map(str::to_owned) {
            change["kind"] = json!(
                target
                    .dependencies
                    .get(&id)
                    .or_else(|| baseline.dependencies.get(&id))
                    .map(|o| &o.kind)
            );
        }
        if let Some(id) = change["dependency"].as_str().map(str::to_owned) {
            changes.extend(scope_changes(&id, baseline, target));
            // Return changed normalized fields, not unchanged catalogues.
            // Inventory changes already have separate file-level records.
            let before = baseline
                .dependencies
                .get(&id)
                .map(|d| d.normalized.clone())
                .unwrap_or(Value::Null);
            let after = target
                .dependencies
                .get(&id)
                .map(|d| d.normalized.clone())
                .unwrap_or(Value::Null);
            let (before, after) =
                if let (Some(a), Some(b)) = (before.as_object(), after.as_object()) {
                    let keys = a
                        .keys()
                        .chain(b.keys())
                        .filter(|k| !(change["kind"] == "SOURCE_SCOPE" && *k == "inventory"))
                        .collect::<BTreeSet<_>>();
                    let keys = keys
                        .into_iter()
                        .filter(|k| a.get(*k) != b.get(*k))
                        .collect::<Vec<_>>();
                    let added = keys
                        .iter()
                        .filter(|k| !a.contains_key(**k))
                        .cloned()
                        .collect::<Vec<_>>();
                    let removed = keys
                        .iter()
                        .filter(|k| !b.contains_key(**k))
                        .cloned()
                        .collect::<Vec<_>>();
                    if !added.is_empty() {
                        change["addedFields"] = json!(added);
                    }
                    if !removed.is_empty() {
                        change["removedFields"] = json!(removed);
                    }
                    (
                        json!(
                            keys.iter()
                                .filter_map(|k| a.get(*k).map(|v| ((*k).clone(), v.clone())))
                                .collect::<BTreeMap<_, _>>()
                        ),
                        json!(
                            keys.iter()
                                .filter_map(|k| b.get(*k).map(|v| ((*k).clone(), v.clone())))
                                .collect::<BTreeMap<_, _>>()
                        ),
                    )
                } else {
                    (before, after)
                };
            change["beforeValues"] = before;
            change["afterValues"] = after;
            change["valueRepresentation"] =
                json!("CHANGED_NORMALIZED_FIELDS_ONLY_INVENTORY_DELTAS_SEPARATE");
        }
        changes.push(change);
    }
    for id in new
        .dependencies
        .keys()
        .filter(|id| !old.dependencies.contains_key(*id))
    {
        changes.push(json!({"reason":"DEPENDENCY_CLOSURE_ADDED","dependency":id}));
    }
    let before = baseline.sources();
    let after = target.sources();
    for id in &old.source_ids {
        match (before.get(id),after.get(id)) {
            (Some(a),Some(b)) if a.text_digest!=b.text_digest||a.file!=b.file=>changes.push(json!({"reason":"SELECTED_SOURCE_BYTES_CHANGED","source":id,"before":source_identity(a),"after":source_identity(b),"changedWindow":changed_window(a,b)})),
            (_,None)=>{missing=true;changes.push(json!({"reason":"SELECTED_SOURCE_UNAVAILABLE","source":id}));},_=>{}
        }
    }
    for (id, file) in &old.native.files {
        if new
            .native
            .files
            .get(id)
            .is_none_or(|f| native_identity(f) != native_identity(file))
        {
            changes.push(json!({"reason":"REGISTERED_NATIVE_FILE_CHANGED","input":id,"before":file,"after":new.native.files.get(id)}));
        }
    }
    for (id, names) in &old.native.membership {
        if new.native.membership.get(id) != Some(names) {
            changes.push(json!({"reason":"REGISTERED_NATIVE_MEMBERSHIP_CHANGED","input":id,"before":names,"after":new.native.membership.get(id)}));
        }
    }
    let changed = changes.iter().any(|v| {
        !matches!(
            v["reason"].as_str(),
            Some("DEPENDENCY_UNAVAILABLE" | "SELECTED_SOURCE_UNAVAILABLE")
        )
    });
    let status = if changed {
        Verdict::Changed
    } else if missing || !old.native.limits.is_empty() || !new.native.limits.is_empty() {
        Verdict::Unknown
    } else {
        Verdict::Unchanged
    };
    (status, changes)
}
pub(super) fn compare(
    repo: &Repository,
    input: &Path,
    snapshot: &str,
    include_context: bool,
) -> Result<Value, ClewError> {
    let supplied: Value = store::read(input, 32 * 1024 * 1024)?;
    let envelope = supplied.get("evidenceSet").unwrap_or(&supplied);
    if envelope["schema"] != SCHEMA {
        return Err(invalid("input has no engine-retained SOURCE evidence set"));
    }
    let reference: cache::ObjectRef =
        serde_json::from_value(envelope["reference"].clone()).map_err(io_error)?;
    if reference.schema != SCHEMA {
        return Err(invalid("SOURCE evidence reference schema differs"));
    }
    let old: Evidence = cache::get_json(repo, &reference, MAX_BYTES)?.ok_or_else(|| {
        invalid(
            "SOURCE evidence set is unavailable in this managed root; use the shared prepared root",
        )
    })?;
    if old.schema != SCHEMA
        || envelope["snapshot"] != old.snapshot
        || envelope["service"] != old.service
        || envelope["symbol"] != old.symbol
    {
        return Err(invalid("SOURCE evidence ticket binding differs"));
    }
    let baseline = Check::load_snapshot(repo, &old.snapshot)?;
    let target = Check::load_snapshot(repo, snapshot)?;
    if config(&baseline, &old.service)? != old.analysis_config {
        return Err(invalid(
            "SOURCE evidence configuration does not bind its snapshot",
        ));
    }
    let original = frame(
        repo,
        &baseline,
        &old.snapshot,
        &old.service,
        &old.symbol,
        old.native_scopes.clone(),
        old.native.clone(),
    )?;
    if original.dependencies != old.dependencies || original.source_ids != old.source_ids {
        return Err(invalid(
            "SOURCE evidence selector/dependency binding differs from its snapshot",
        ));
    }
    let next_config = config(&target, &old.service)?;
    if next_config["repositoryId"] != old.analysis_config["repositoryId"]
        || next_config["repository"] != old.analysis_config["repository"]
    {
        return Err(invalid("SOURCE comparison repository identity differs"));
    }
    // Read every registered old object even if its text will not be delivered.
    let old_native_items = native_items(repo, &old.native, &baseline)?;
    validate_scopes(&old.native_scopes)?;
    let same_native_revisions = old.native_scopes.iter().all(|s| {
        let (Some(a), Some(b)) = (
            baseline.services.get(&s.service),
            target.services.get(&s.service),
        ) else {
            return false;
        };
        let (Ok(ac), Ok(bc)) = (config(&baseline, &s.service), config(&target, &s.service)) else {
            return false;
        };
        a.revision == b.revision
            && ac["repositoryId"] == bc["repositoryId"]
            && ac["repository"] == bc["repository"]
    });
    let native = if same_native_revisions {
        old.native.clone()
    } else {
        capture_native(repo, &target, &old.native_scopes)?
    };
    let next = match frame(
        repo,
        &target,
        snapshot,
        &old.service,
        &old.symbol,
        old.native_scopes.clone(),
        native.clone(),
    ) {
        Ok(next) => next,
        Err(error) => {
            let mut result = packet(repo, &baseline, &old, &reference, false)?;
            result["snapshot"] = json!(snapshot);
            result["inputDigest"] = json!(target.input_digest);
            result["contextDigest"] = json!(target.context_digest);
            result["comparison"] = json!({"status":Verdict::Unknown,"reason":"TARGET_CONTEXT_UNAVAILABLE","detailsCode":error.code,"scope":"REGISTERED_INPUTS_ONLY_NOT_RUNTIME_BEHAVIOR","baselineSnapshot":old.snapshot,"targetSnapshot":snapshot,"behaviorEquivalence":"NOT_ESTABLISHED","unresolvedSemanticsPreserved":true});
            result["targetNativeInputLimits"] = json!(native.limits);
            result["targetUnresolved"] = json!(target.unresolved);
            result["comparison"]["baselineBinding"] = maintenance_binding(&baseline, &old)?;
            result["comparison"]["targetBinding"] = json!({"snapshot":snapshot,"service":old.service,"symbol":old.symbol,"inputDigest":target.input_digest,"contextDigest":target.context_digest,"revision":target.services.get(&old.service).map(|s|&s.revision),"analysisConfigurationDigest":digest(&next_config)?});
            return Ok(result);
        }
    };
    let (status, mut changes) = compare_frames(&old, &next, &baseline, &target);
    let new_native_items = native_items(repo, &next.native, &target)?;
    for change in &mut changes {
        if change["reason"] == "REGISTERED_NATIVE_FILE_CHANGED" {
            fn find<'a>(items: &'a [Value], file: &Value) -> Option<&'a Value> {
                items
                    .iter()
                    .map(|i| &i["record"])
                    .find(|r| r["service"] == file["service"] && r["file"] == file["path"])
            }
            if let (Some(a), Some(b)) = (
                find(&old_native_items, &change["before"]),
                find(&new_native_items, &change["after"]),
            ) {
                let identity = |v: &Value| {
                    let mut v = v.clone();
                    v.as_object_mut().unwrap().remove("text");
                    v
                };
                if let (Some(before), Some(after)) = (a["text"].as_str(), b["text"].as_str()) {
                    change["changedWindow"] = text_window(before, after, identity(a), identity(b));
                }
            }
        }
    }
    let next_ref = store_evidence(repo, &next)?;
    let mut result = packet(
        repo,
        &target,
        &next,
        &next_ref,
        include_context || status != Verdict::Unchanged,
    )?;
    let old_sources = baseline.sources();
    let new_sources = target.sources();
    let mut links=old.source_ids.iter().filter_map(|id|Some((id,old_sources.get(id)?,new_sources.get(id)?))).filter(|(_,a,b)|a.revision!=b.revision||a.url!=b.url||a.start_line!=b.start_line||a.end_line!=b.end_line).map(|(id,a,b)|json!({"source":id,"before":source_identity(a),"after":source_identity(b),"meaning":"PROVENANCE_OR_LOCATION_ONLY_UNLESS_SEPARATELY_LISTED_AS_BYTE_CHANGE"})).collect::<Vec<_>>();
    for after in new_native_items {
        if let Some(before) = old_native_items.iter().find(|v| v["id"] == after["id"])
            && (before["record"]["revision"] != after["record"]["revision"]
                || before["record"]["url"] != after["record"]["url"])
        {
            let identity = |v: &Value| {
                let mut v = v["record"].clone();
                v.as_object_mut().unwrap().remove("text");
                v
            };
            links.push(json!({"source":after["id"],"before":identity(before),"after":identity(&after),"meaning":"REGISTERED_NATIVE_BYTE_BINDING_PROVENANCE_ONLY_UNLESS_SEPARATELY_CHANGED"}));
        }
    }
    let selected_source_bytes = if changes
        .iter()
        .any(|c| c["reason"] == "SELECTED_SOURCE_UNAVAILABLE")
    {
        Verdict::Unknown
    } else if changes
        .iter()
        .any(|c| c["reason"] == "SELECTED_SOURCE_BYTES_CHANGED")
    {
        Verdict::Changed
    } else {
        Verdict::Unchanged
    };
    result["comparison"] = json!({"status":status,"scope":"REGISTERED_INPUTS_ONLY_NOT_RUNTIME_BEHAVIOR","integrity":"BOTH_IMMUTABLE_SNAPSHOT_CLOSURES_ADMITTED","baselineSnapshot":old.snapshot,"targetSnapshot":snapshot,"changeCount":changes.len(),"sourceBindingUpdateCount":links.len(),"selectedSourceBytes":selected_source_bytes,"semanticCompleteness":"NOT_ESTABLISHED","reasonExamples":changes.iter().take(4).map(|v|json!({"reason":v["reason"],"kind":v["kind"],"dependency":v["dependency"],"source":v["source"],"input":v["input"],"file":v["file"],"changedWindow":v["changedWindow"],"sourceReferences":v["sourceReferences"]})).collect::<Vec<_>>(),"unresolvedSemanticsPreserved":true,"unregisteredReferences":"UNKNOWN_OUT_OF_SCOPE","behaviorEquivalence":"NOT_ESTABLISHED"});
    result["comparison"]["baselineBinding"] = maintenance_binding(&baseline, &old)?;
    result["comparison"]["targetBinding"] = maintenance_binding(&target, &next)?;
    let mut rows=changes.into_iter().enumerate().map(|(i,record)|json!({"kind":"EVIDENCE_CHANGE","id":format!("change-{i}"),"record":record})).collect::<Vec<_>>();
    rows.extend(links.into_iter().enumerate().map(|(i,record)|json!({"kind":"SOURCE_BINDING_UPDATE","id":format!("binding-{i}"),"record":record})));
    rows.extend(std::mem::take(result["items"].as_array_mut().unwrap()));
    result["items"] = json!(rows);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Evidence, Check) {
        let hash = crate::canonical::hash_bytes(b"handler");
        let mut check:Check=serde_json::from_value(json!({"schema":"check","inputDigest":"inputs","contextDigest":"context","services":{"sample":{"schema":"evidence","service":"sample","revision":"old","serviceDigest":"digest","extractor":"syntax","runtimeMode":"pinned","coverage":"SYNTAX","boundaries":["CALL_TARGETS_UNRESOLVED"],"entrypoints":[],"observations":{},"contracts":{},"sources":{"source":{"id":"source","service":"sample","revision":"old","file":"main.ts","startLine":1,"endLine":1,"text":"handler","textDigest":hash,"evidenceDigest":"evidence","authority":"EXACT_SNAPSHOT_TEXT","url":null}}}},"unresolved":{},"interactions":{},"scenarios":{},"dependencies":{}})).unwrap();
        for (id, kind) in [
            ("handler", "SYMBOL"),
            ("callee", "SYMBOL"),
            ("scope", "SOURCE_SCOPE"),
        ] {
            check.dependencies.insert(id.into(),serde_json::from_value(json!({"id":id,"service":"sample","kind":kind,"symbol":id,"normalized":{},"digest":format!("digest-{id}"),"sourceIds":["source"]})).unwrap());
        }
        let evidence = Evidence {
            schema: SCHEMA.into(),
            snapshot: "baseline".into(),
            service: "sample".into(),
            symbol: "handler".into(),
            analysis_config: json!({"repositoryId":"repo","source":{"roots":["src"]}}),
            dependencies: check
                .dependencies
                .iter()
                .map(|(k, v)| (k.clone(), v.digest.clone()))
                .collect(),
            source_ids: vec!["source".into()],
            native_scopes: vec![],
            native: NativeState::default(),
        };
        (evidence, check)
    }
    #[test]
    fn same_inputs_and_revision_only_changes_retain_syntax_limits_without_runtime_claims() {
        let (old, check) = fixture();
        let mut target = check.clone();
        target
            .services
            .get_mut("sample")
            .unwrap()
            .sources
            .get_mut("source")
            .unwrap()
            .revision = "new".into();
        assert_eq!(
            compare_frames(&old, &old, &check, &target).0,
            Verdict::Unchanged
        );
        let dir = tempfile::tempdir().unwrap();
        Repository::init(dir.path(), "test").unwrap();
        let repo = Repository::open(dir.path()).unwrap();
        let reference = store_evidence(&repo, &old).unwrap();
        let result = packet(&repo, &target, &old, &reference, false).unwrap();
        assert_eq!(result["authority"], "PINNED_SNAPSHOT_NOT_REVERIFIED");
        assert_eq!(
            result["evidenceValidation"]["behaviorEquivalence"],
            "NOT_ESTABLISHED"
        );
        assert_eq!(
            result["registeredInputLimits"]["coverage"]["boundaries"],
            json!(["CALL_TARGETS_UNRESOLVED"])
        );
    }
    #[test]
    fn changed_handler_callee_generated_config_and_membership_never_return_unchanged() {
        let (old, check) = fixture();
        for id in ["handler", "callee", "scope"] {
            let mut target = check.clone();
            target.dependencies.get_mut(id).unwrap().digest = "changed".into();
            let mut new = old.clone();
            new.dependencies.insert(id.into(), "changed".into());
            assert_eq!(
                compare_frames(&old, &new, &check, &target).0,
                Verdict::Changed,
                "{id}"
            );
        }
        let mut new = old.clone();
        new.analysis_config["source"]["generatedInputs"] =
            json!([{"path":"generated.ts","origin":"generator-v2"}]);
        assert_eq!(
            compare_frames(&old, &new, &check, &check).0,
            Verdict::Changed
        );
        let mut old = old;
        old.native
            .membership
            .insert("sample:models".into(), json!(["models/a.cs"]));
        let mut new = old.clone();
        new.native.membership.insert(
            "sample:models".into(),
            json!(["models/a.cs", "models/b.cs"]),
        );
        assert_eq!(
            compare_frames(&old, &new, &check, &check).0,
            Verdict::Changed
        );
    }
    #[test]
    fn missing_dependency_source_and_native_input_are_explicit_unknown() {
        let (old, check) = fixture();
        let mut target = check.clone();
        target.dependencies.remove("callee");
        let mut new = old.clone();
        new.dependencies.remove("callee");
        let (status, reasons) = compare_frames(&old, &new, &check, &target);
        assert_eq!(status, Verdict::Unknown);
        assert_eq!(reasons[0]["reason"], "DEPENDENCY_UNAVAILABLE");
        let mut target = check.clone();
        target.services.get_mut("sample").unwrap().sources.clear();
        assert_eq!(
            compare_frames(&old, &old, &check, &target).0,
            Verdict::Unknown
        );
        let mut new = old.clone();
        new.native
            .limits
            .push(json!({"reason":"NATIVE_INPUT_UNAVAILABLE"}));
        assert_eq!(
            compare_frames(&old, &new, &check, &check).0,
            Verdict::Unknown
        );
    }
    #[test]
    fn registered_receiver_bytes_change_without_upgrading_native_authority() {
        let (mut old, check) = fixture();
        let file = NativeFile {
            service: "sample".into(),
            path: "Receiver.cs".into(),
            revision: "old".into(),
            mode: "100644".into(),
            blob: "blob".into(),
            status: "AVAILABLE".into(),
            content: Some(cache::ObjectRef::new(
                NATIVE_TEXT.into(),
                "sha256:old".into(),
                3,
            )),
        };
        old.native.files.insert("sample:Receiver.cs".into(), file);
        let mut new = old.clone();
        new.native
            .files
            .get_mut("sample:Receiver.cs")
            .unwrap()
            .revision = "new".into();
        assert_eq!(
            compare_frames(&old, &new, &check, &check).0,
            Verdict::Unchanged
        );
        new.native
            .files
            .get_mut("sample:Receiver.cs")
            .unwrap()
            .content
            .as_mut()
            .unwrap()
            .digest = "sha256:new".into();
        let (status, reasons) = compare_frames(&old, &new, &check, &check);
        assert_eq!(status, Verdict::Changed);
        assert_eq!(reasons[0]["reason"], "REGISTERED_NATIVE_FILE_CHANGED");
    }
    #[test]
    fn scope_delta_distinguishes_changed_file_and_explicit_text_projection() {
        let (old, mut check) = fixture();
        check.dependencies.get_mut("scope").unwrap().normalized =
            json!({"inventory":{"index.html":{"digest":"old","coverage":"FILE_ONLY"}}});
        check
            .services
            .get_mut("sample")
            .unwrap()
            .sources
            .get_mut("source")
            .unwrap()
            .file = "index.html".into();
        check
            .services
            .get_mut("sample")
            .unwrap()
            .sources
            .get_mut("source")
            .unwrap()
            .text = "<title>Old</title>".into();
        {
            let source = check
                .services
                .get_mut("sample")
                .unwrap()
                .sources
                .get_mut("source")
                .unwrap();
            source.start_line = 42;
            source.end_line = 42;
            source.url = Some("https://example.test/blob/old/index.html#L42-L42".into());
        }
        let mut target = check.clone();
        {
            let source = target
                .services
                .get_mut("sample")
                .unwrap()
                .sources
                .get_mut("source")
                .unwrap();
            source.revision = "new".into();
            source.start_line = 43;
            source.end_line = 43;
            source.url = Some("https://example.test/blob/new/index.html#L43-L43".into());
        }
        target.dependencies.get_mut("scope").unwrap().normalized["inventory"]["index.html"]["digest"] =
            json!("new");
        target.dependencies.get_mut("scope").unwrap().digest = "changed".into();
        target
            .services
            .get_mut("sample")
            .unwrap()
            .sources
            .get_mut("source")
            .unwrap()
            .text = "<title>New</title>".into();
        let (status, changes) = compare_frames(&old, &old, &check, &target);
        assert_eq!(status, Verdict::Changed);
        let delta = changes
            .iter()
            .find(|c| c["reason"] == "REGISTERED_SOURCE_SCOPE_FILE_CHANGED")
            .unwrap();
        assert_eq!(delta["file"], "index.html");
        assert_eq!(delta["meaningImpact"], "NOT_INFERRED");
        assert_eq!(delta["changedWindow"]["after"], "<title>New</title>");
        for (side, source) in [
            (
                "beforeCitation",
                &check.services["sample"].sources["source"],
            ),
            (
                "afterCitation",
                &target.services["sample"].sources["source"],
            ),
        ] {
            assert_eq!(delta["changedWindow"][side], source_identity(source));
            assert!(delta["changedWindow"][side].get("text").is_none());
        }
        assert_eq!(delta["changedWindow"]["beforeCitation"]["startLine"], 42);
        assert_eq!(delta["changedWindow"]["afterCitation"]["revision"], "new");
        // No source was returned for a missing file: never invent citation bindings.
        assert!(file_excerpt(&check, "sample", "missing.html").is_none());
        assert!(
            delta["changedWindow"]["representation"]
                .as_str()
                .unwrap()
                .contains("PROJECTION")
        );
    }
    #[test]
    fn maintenance_reports_callee_values_and_selected_text_without_source_replay() {
        let (old, baseline) = fixture();
        let mut target = baseline.clone();
        let callee = target.dependencies.get_mut("callee").unwrap();
        callee.normalized = json!({"retryLimit":2});
        callee.digest = "changed".into();
        let (status, changes) = compare_frames(&old, &old, &baseline, &target);
        assert_eq!(status, Verdict::Changed);
        assert!(
            changes
                .iter()
                .all(|c| c["reason"] != "SELECTED_SOURCE_BYTES_CHANGED")
        );
        let delta = changes
            .iter()
            .find(|c| c["dependency"] == "callee")
            .unwrap();
        assert_eq!(delta["beforeValues"], json!({}));
        assert_eq!(delta["afterValues"], json!({"retryLimit":2}));
        assert_eq!(delta["addedFields"], json!(["retryLimit"]));
        let source = target
            .services
            .get_mut("sample")
            .unwrap()
            .sources
            .get_mut("source")
            .unwrap();
        source.text = "new λ🙂 handler".into();
        source.text_digest = crate::canonical::hash_bytes(source.text.as_bytes());
        let (_, changes) = compare_frames(&old, &old, &baseline, &target);
        let delta = changes
            .iter()
            .find(|c| c["reason"] == "SELECTED_SOURCE_BYTES_CHANGED")
            .unwrap();
        assert_eq!(delta["changedWindow"]["before"], "handler");
        assert_eq!(delta["changedWindow"]["after"], "new λ🙂 handler");
        assert_eq!(
            delta["changedWindow"]["afterCitation"]["textDigest"],
            target.services["sample"].sources["source"].text_digest
        );
        let binding = maintenance_binding(&baseline, &old).unwrap();
        assert_eq!(binding["snapshot"], "baseline");
        assert_eq!(binding["revision"], "old");
        assert_eq!(
            binding["analysisConfigurationDigest"],
            digest(&old.analysis_config).unwrap()
        );
    }

    #[test]
    fn native_registration_rejects_unsafe_scopes_and_missing_retained_bytes() {
        for path in ["../secret", "src/*.cs", "src/file\nother", "/absolute"] {
            assert!(
                validate_scopes(&[NativeScope {
                    service: "sample".into(),
                    files: vec![path.into()],
                    membership: vec![]
                }])
                .is_err()
            );
        }
        let dir = tempfile::tempdir().unwrap();
        Repository::init(dir.path(), "test").unwrap();
        let repo = Repository::open(dir.path()).unwrap();
        let mut state = NativeState::default();
        state.files.insert(
            "sample:file".into(),
            NativeFile {
                service: "sample".into(),
                path: "file".into(),
                revision: "revision".into(),
                mode: "100644".into(),
                blob: "blob".into(),
                status: "AVAILABLE".into(),
                content: Some(cache::ObjectRef::new(
                    NATIVE_TEXT.into(),
                    format!("sha256:{}", "0".repeat(64)),
                    3,
                )),
            },
        );
        assert!(native_items(&repo, &state, &fixture().1).is_err());
    }
}
