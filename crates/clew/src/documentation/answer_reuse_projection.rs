//! Read-only replay of the supported initial operation evidence preparation.
use super::{
    bindings,
    check::Check,
    digest, invalid, operation_answer, operation_packet,
    store::Repository,
    work::{self, Request, Work},
};
use crate::error::ClewError;
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub(super) fn normalized_request(
    repo: &Repository,
    subject: &str,
    request: Request,
    current: &Check,
) -> Result<Request, ClewError> {
    let language = if request.documentation_language.is_none() {
        bindings::baseline(repo)?.and_then(|(_, binding)| binding.documentation_language)
    } else {
        None
    };
    work::normalize_replay_request(subject, request, current, language)
}

/// A strict pin delta may be provenance-only only after the full current
/// preparation has been reconstructed and compared successfully. Unknown or
/// missing bindings and source-content deltas never receive this exception.
pub(super) fn selected_provenance_only(delta: &Value, replay: &Value) -> bool {
    replay["status"] == "CURRENT"
        && delta["status"] == "STALE"
        && delta["changedSources"]
            .as_array()
            .is_some_and(Vec::is_empty)
        && delta["missingPinsOrBindings"]
            .as_array()
            .is_some_and(Vec::is_empty)
        && delta["changedPins"].as_array().is_some_and(|pins| {
            !pins.is_empty()
                && pins
                    .iter()
                    .all(|pin| pin["reason"] == "SELECTED_DEPENDENCY_CHANGED")
        })
}

fn supported(saved: &Work) -> Result<(), ClewError> {
    if !saved.subject.starts_with("service:")
        || saved.request.context_profile.as_deref() != Some("process-graph-v1")
        || saved.request.authoring_contract.as_deref()
            != Some(operation_answer::EXPANDING_AUTHORING_CONTRACT)
        || !saved.request.source_data_context
        || saved.request.entrypoint.is_some()
        || saved.request.maintained_paragraph.is_some()
        || saved.request.maintained_from_bundle.is_some()
        || !saved.request.external_inputs.is_empty()
        || saved.maintained_context.is_some()
        || saved.retained.is_some()
        || saved.external_inputs != BTreeMap::from([("notes".into(), json!({"status":"ABSENT"}))])
    {
        return Err(invalid(
            "ANSWER_REUSE_UNSUPPORTED: supported replay requires service process-graph-v1, authoring/1.6, sourceDataContext and absent notes, without maintained, retained or external context",
        ));
    }
    let root =
        super::process_graph::resolve_work_root(&saved.subject, &saved.request, &saved.checked)?;
    if root.declaration.normalized["declarationKind"] != "METHOD" {
        return Err(invalid(
            "ANSWER_REUSE_UNSUPPORTED: replay supports compiler methods only",
        ));
    }
    Ok(())
}

pub(super) fn compare(
    repo: &Repository,
    saved: &Work,
    current: &Check,
) -> Result<Value, ClewError> {
    supported(saved)?;
    let service = saved.subject.strip_prefix("service:").unwrap_or_default();
    for checked in [&saved.checked, current] {
        let expectation = checked
            .source_inputs
            .as_ref()
            .is_some_and(|inputs| inputs.inputs.evidence_expectations.contains_key(service));
        let package = checked
            .dependencies
            .values()
            .any(|record| record.service == service && record.kind == "EVIDENCE_PACKAGE")
            || checked.services.get(service).is_some_and(|evidence| {
                evidence
                    .observations
                    .values()
                    .any(|record| record.kind == "EVIDENCE_PACKAGE")
            });
        if expectation || package {
            return Err(invalid(
                "ANSWER_REUSE_UNSUPPORTED: portable evidence expectations and EVIDENCE_PACKAGE producer authority are outside native initial replay",
            ));
        }
    }
    // A metadata probe is not a capture. Check freezes associated notes only,
    // so it cannot establish absence of the complete protected notes tree.
    match std::fs::symlink_metadata(repo.path("notes")?) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Ok(_) => return Ok(json!({"status":"STALE","reasons":["NOTES_MEMBERSHIP_CHANGED"]})),
        Err(_) => {
            return Err(invalid(
                "ANSWER_REUSE_NOTES_UNAVAILABLE: protected notes membership cannot be established",
            ));
        }
    }
    if current
        .source_inputs
        .as_ref()
        .is_some_and(|inputs| !inputs.inputs.notes.is_empty())
        || current
            .dependencies
            .values()
            .any(|dependency| dependency.kind == "NOTE_ASSOCIATION")
    {
        return Ok(json!({"status":"STALE","reasons":["NOTES_MEMBERSHIP_CHANGED"]}));
    }
    compare_loaded(saved, current)
}

fn compare_loaded(saved: &Work, current: &Check) -> Result<Value, ClewError> {
    let replay = work::replay_process_work(saved, current)?;
    let historical = projection(saved)?;
    let compared = projection(&replay)?;
    let mut reasons = Vec::new();
    for (key, reason) in [
        ("records", "INITIAL_SELECTION_CHANGED"),
        ("packet", "INITIAL_PACKET_CHANGED"),
        ("graph", "PROCESS_GRAPH_CHANGED"),
        ("producer", "PRODUCER_OR_SELECTION_CHANGED"),
    ] {
        if historical[key] != compared[key] {
            reasons.push(reason);
        }
    }
    Ok(
        json!({"status":if reasons.is_empty(){"CURRENT"}else{"STALE"},
        "reasons":reasons,"historicalProjectionDigest":digest(&historical)?,
        "comparedProjectionDigest":digest(&compared)?,
        "selectedIdentities":compared["records"].as_object().map(|records| records.keys().collect::<Vec<_>>()).unwrap_or_default()}),
    )
}

fn identity(kind: &str, id: &str) -> String {
    // JSON tuple encoding avoids delimiter collisions in semantic identities.
    json!([kind, id]).to_string()
}

fn projection(work: &Work) -> Result<Value, ClewError> {
    let (mut packet, audit) = operation_packet::build(work)?;
    let mut references: BTreeMap<String, String> = work
        .handles
        .iter()
        .map(|(reference, handle)| (reference.clone(), identity(&handle.kind, &handle.id)))
        .collect();
    let mut records = BTreeMap::new();
    for record in audit["records"]
        .as_array()
        .ok_or_else(|| invalid("ANSWER_REUSE_PROJECTION_INVALID: selected records are missing"))?
    {
        let kind = record["kind"]
            .as_str()
            .ok_or_else(|| invalid("selected kind missing"))?;
        let id = record["id"]
            .as_str()
            .ok_or_else(|| invalid("selected identity missing"))?;
        let key = identity(kind, id);
        if kind == "DEPENDENCY" {
            validate_source_sites(work, &record["row"]["record"])?;
        }
        let label = record["label"]
            .as_str()
            .ok_or_else(|| invalid("selected label missing"))?;
        if references
            .insert(label.into(), key.clone())
            .is_some_and(|old| old != key)
        {
            return Err(invalid(
                "ANSWER_REUSE_PROJECTION_INVALID: conflicting semantic label",
            ));
        }
        if records.insert(key, record["row"].clone()).is_some() {
            return Err(invalid(
                "ANSWER_REUSE_PROJECTION_INVALID: duplicate selected identity",
            ));
        }
    }
    let mut spans = BTreeMap::new();
    for (id, binding) in packet["sourceDataContext"]["sourceSpans"]
        .as_object()
        .ok_or_else(|| {
            invalid("ANSWER_REUSE_PROJECTION_INVALID: source span bindings are missing")
        })?
    {
        let reference = binding["reference"]
            .as_str()
            .and_then(|value| references.get(value))
            .ok_or_else(|| invalid("ANSWER_REUSE_PROJECTION_INVALID: unresolved source span"))?;
        spans.insert(
            id.clone(),
            identity(
                "SOURCE_SPAN",
                &json!([
                    reference,
                    binding["startByte"],
                    binding["endByte"],
                    binding["textDigest"]
                ])
                .to_string(),
            ),
        );
    }
    packet.as_object_mut().unwrap().remove("packetDigest");
    packet["graphAuditBinding"]
        .as_object_mut()
        .ok_or_else(|| invalid("graph binding missing"))?
        .remove("artifactDigest");
    packet["graphAuditBinding"]
        .as_object_mut()
        .unwrap()
        .remove("snapshot");
    for edge in packet["edges"]
        .as_array_mut()
        .ok_or_else(|| invalid("packet edges missing"))?
    {
        if let Some(reference) = edge["id"].as_str().and_then(|id| id.strip_prefix("fact:")) {
            let stable = references.get(reference).ok_or_else(|| {
                invalid("ANSWER_REUSE_PROJECTION_INVALID: unknown provider edge reference")
            })?;
            edge["id"] = json!(format!("fact:{stable}"));
        }
    }
    let data = packet["sourceDataContext"].as_object_mut().unwrap();
    for key in ["snapshot", "sourceDataDigest", "examinedSourceDigest"] {
        data.remove(key);
    }
    for node in data
        .get_mut("nodes")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| invalid("source-data nodes missing"))?
    {
        let object = node
            .as_object_mut()
            .ok_or_else(|| invalid("source-data node invalid"))?;
        object.remove("examinedSourceDigest");
        object.remove("variableFactsDigest");
        object
            .get_mut("dataState")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| invalid("source-data state missing"))?
            .remove("dataStateDigest");
    }
    for binding in data
        .get_mut("sourceSpans")
        .and_then(Value::as_object_mut)
        .unwrap()
        .values_mut()
    {
        binding.as_object_mut().unwrap().remove("sourceDigest");
    }
    let mut graph = audit["processGraph"].clone();
    graph
        .as_object_mut()
        .ok_or_else(|| invalid("process graph missing"))?
        .remove("artifactDigest");
    // These fields bind whole captures, rather than the replayed graph content.
    graph["snapshot"] = json!({"service":graph["snapshot"]["service"]});
    for source in graph["sources"]
        .as_object_mut()
        .ok_or_else(|| invalid("process graph sources missing"))?
        .values_mut()
    {
        normalize_source(source)?;
    }
    let mut records = serde_json::to_value(records).map_err(super::io_error)?;
    for row in records.as_object_mut().unwrap().values_mut() {
        if row["kind"] == "SOURCE" {
            normalize_source(&mut row["record"])?;
        }
        if row["kind"] == "COVERAGE" {
            row["record"].as_object_mut().unwrap().remove("revision");
        }
        normalize(row, "", &references, &spans)?;
    }
    normalize(&mut packet, "", &references, &spans)?;
    normalize(&mut graph, "", &references, &spans)?;
    let service = work.subject.strip_prefix("service:").unwrap_or_default();
    let evidence = work
        .checked
        .services
        .get(service)
        .ok_or_else(|| invalid("replay service missing"))?;
    let mut selection = work
        .checked
        .source_inputs
        .as_ref()
        .and_then(|inputs| inputs.inputs.services.get(service))
        .map(serde_json::to_value)
        .transpose()
        .map_err(super::io_error)?
        .unwrap_or(Value::Null);
    if let Some(object) = selection.as_object_mut() {
        object.remove("targetRef");
    }
    Ok(json!({"records":records,"packet":packet,"graph":graph,
        "producer":{"schema":evidence.schema,"extractor":evidence.extractor,"runtimeMode":evidence.runtime_mode,"selection":selection,"selectionGuidance":super::agent_jobs::selection_guidance(work)}}))
}

fn normalize_source(source: &mut Value) -> Result<(), ClewError> {
    // Only explicitly typed source records enter this function.
    let typed: super::model::Source =
        serde_json::from_value(source.clone()).map_err(super::io_error)?;
    if crate::canonical::hash_bytes(typed.text.as_bytes()) != typed.text_digest {
        return Err(invalid(
            "ANSWER_REUSE_SOURCE_CORRUPT: selected text digest differs",
        ));
    }
    let object = source.as_object_mut().unwrap();
    for key in ["revision", "evidenceDigest", "url"] {
        object.remove(key);
    }
    if let Some(occurrence) = object.get_mut("occurrence").and_then(Value::as_object_mut) {
        occurrence.remove("snapshot");
    }
    Ok(())
}

fn validate_source_sites(work: &Work, observation: &Value) -> Result<(), ClewError> {
    let normalized = &observation["normalized"];
    if normalized["schema"] != "codeclew-java-compiler-fact/1.0" {
        return Ok(());
    }
    for key in ["variableSite", "callSite", "declarationSite"] {
        let site = &normalized[key];
        if site["sourceStatus"] != "SOURCE_RETAINED" {
            continue;
        }
        let source = site["sourceId"]
            .as_str()
            .and_then(|id| {
                work.checked
                    .services
                    .values()
                    .find_map(|evidence| evidence.sources.get(id))
            })
            .ok_or_else(|| {
                invalid("ANSWER_REUSE_SOURCE_BINDING_CORRUPT: retained compiler site has no source")
            })?;
        if site["sourceDigest"] != source.text_digest
            || site["evidenceDigest"] != source.evidence_digest
            || site["file"] != source.file
            || (site.get("service").is_some() && site["service"] != source.service)
            || (site.get("revision").is_some() && site["revision"] != source.revision)
            || crate::canonical::hash_bytes(source.text.as_bytes()) != source.text_digest
        {
            return Err(invalid(
                "ANSWER_REUSE_SOURCE_BINDING_CORRUPT: compiler site differs from retained source",
            ));
        }
        if site.get("sourceByteStart").is_some() || site.get("sourceByteEnd").is_some() {
            let origin = site["sourceByteStart"].as_u64();
            let limit = site["sourceByteEnd"].as_u64();
            let range = origin
                .zip(limit)
                .filter(|(a, b)| b.checked_sub(*a) == Some(source.text.len() as u64));
            if range.is_none() {
                return Err(invalid(
                    "ANSWER_REUSE_SOURCE_BINDING_CORRUPT: compiler source range differs",
                ));
            }
            if site.get("spanDigest").is_some() {
                let origin = range.unwrap().0;
                let span = site["byteStart"]
                    .as_u64()
                    .zip(site["byteEnd"].as_u64())
                    .and_then(|(a, b)| a.checked_sub(origin).zip(b.checked_sub(origin)))
                    .and_then(|(a, b)| usize::try_from(a).ok().zip(usize::try_from(b).ok()))
                    .and_then(|(a, b)| source.text.get(a..b));
                if span.map(|text| crate::canonical::hash_bytes(text.as_bytes()))
                    != site["spanDigest"].as_str().map(str::to_owned)
                {
                    return Err(invalid(
                        "ANSWER_REUSE_SOURCE_BINDING_CORRUPT: compiler variable span differs",
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Remove only typed provenance and derived bindings. Semantic source text and
/// arbitrary normalized values are never subjected to textual replacement.
fn normalize(
    value: &mut Value,
    field: &str,
    references: &BTreeMap<String, String>,
    spans: &BTreeMap<String, String>,
) -> Result<(), ClewError> {
    match value {
        Value::Object(object) => {
            if matches!(field, "normalized" | "annotations" | "metadata" | "trigger") {
                if field == "normalized"
                    && object.get("schema").and_then(Value::as_str)
                        == Some("codeclew-java-compiler-fact/1.0")
                {
                    for key in ["variableSite", "callSite", "declarationSite"] {
                        if let Some(site) = object.get_mut(key).and_then(Value::as_object_mut)
                            && site.contains_key("sourceDigest")
                            && site.contains_key("evidenceDigest")
                        {
                            site.remove("revision");
                            site.remove("evidenceDigest");
                        }
                    }
                }
                return Ok(());
            }
            if object.contains_key("normalized")
                && object.contains_key("sourceIds")
                && object.contains_key("digest")
            {
                if digest(&object["normalized"])? != object["digest"].as_str().unwrap_or_default() {
                    return Err(invalid(
                        "ANSWER_REUSE_DEPENDENCY_CORRUPT: selected observation digest differs",
                    ));
                }
                object.remove("digest");
            }
            if matches!(field, "citations" | "sourceSpans") {
                let original = std::mem::take(object);
                for (key, value) in original {
                    let key = references
                        .get(&key)
                        .or_else(|| spans.get(&key))
                        .cloned()
                        .ok_or_else(|| {
                            invalid("ANSWER_REUSE_PROJECTION_INVALID: unresolved citation identity")
                        })?;
                    if object.insert(key, value).is_some() {
                        return Err(invalid(
                            "ANSWER_REUSE_PROJECTION_INVALID: duplicate citation identity",
                        ));
                    }
                }
            }
            for (key, item) in object.iter_mut() {
                normalize(item, key, references, spans)?;
            }
        }
        Value::Array(items) => {
            for item in items.iter_mut() {
                normalize(item, field, references, spans)?;
            }
            if matches!(
                field,
                "evidence"
                    | "sourceReferences"
                    | "dependencyReferences"
                    | "fieldDeclarations"
                    | "propertyDeclarations"
                    | "contextReferences"
            ) {
                items.sort_by_key(Value::to_string);
            }
            if field == "sources" && items.iter().all(|item| item.get("reference").is_some()) {
                items.sort_by_key(|item| item["reference"].to_string());
            }
        }
        Value::String(text) => {
            if field == "sourceSpan" {
                *text = spans.get(text).cloned().ok_or_else(|| {
                    invalid("ANSWER_REUSE_PROJECTION_INVALID: unknown source span")
                })?;
            } else if (field == "reference"
                || field.ends_with("Reference")
                || field.ends_with("References")
                || matches!(
                    field,
                    "evidence" | "citationId" | "fieldDeclarations" | "propertyDeclarations"
                ))
                && let Some(stable) = references.get(text)
            {
                *text = stable.clone();
            }
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::documentation::model::{Observation, Source};

    fn first_difference(left: &Value, right: &Value, path: &str) -> String {
        if left == right {
            return String::new();
        }
        if let (Some(a), Some(b)) = (left.as_object(), right.as_object()) {
            for key in a.keys().chain(b.keys()) {
                if a.get(key) != b.get(key) {
                    return first_difference(&left[key], &right[key], &format!("{path}/{key}"));
                }
            }
        }
        if let (Some(a), Some(b)) = (left.as_array(), right.as_array()) {
            for index in 0..a.len().max(b.len()) {
                if a.get(index) != b.get(index) {
                    return first_difference(
                        &left[index],
                        &right[index],
                        &format!("{path}/{index}"),
                    );
                }
            }
        }
        format!(
            "{path}: {} != {}",
            left.to_string().chars().take(200).collect::<String>(),
            right.to_string().chars().take(200).collect::<String>()
        )
    }

    fn fixture() -> Work {
        let mut work = work::api_contract_tests::endpoint_context_fixture();
        work.request.entrypoint = None;
        work.request.context_profile = Some("process-graph-v1".into());
        work.request.authoring_contract =
            Some(operation_answer::EXPANDING_AUTHORING_CONTRACT.into());
        work.request.root_declaration = Some("endpoint-declaration".into());
        work.request.question = Some("What happens when request is null?".into());
        work.request.source_data_context = true;
        work.external_inputs = BTreeMap::from([("notes".into(), json!({"status":"ABSENT"}))]);
        let source = work
            .checked
            .services
            .get_mut("orders")
            .unwrap()
            .sources
            .get_mut("endpoint-source")
            .unwrap();
        source.text = "Response handle(Request request) { if (request == null) return null; return service.process(request); }".into();
        source.text_digest = crate::canonical::hash_bytes(source.text.as_bytes());
        work::replay_process_work(&work, &work.checked).unwrap()
    }

    fn add_observation(checked: &mut Check, observation: Observation) {
        checked
            .services
            .get_mut("orders")
            .unwrap()
            .observations
            .insert(observation.id.clone(), observation.clone());
        checked
            .dependencies
            .insert(observation.id.clone(), observation);
    }

    #[test]
    fn replay_ignores_unrelated_membership_and_rebinds_shifted_handles() {
        let saved = fixture();
        let mut current = saved.checked.clone();
        let mut source: Source = current.services["orders"].sources["endpoint-source"].clone();
        source.id = "000-unrelated-source".into();
        source.file = "src/Unrelated.java".into();
        source.text = "class Unrelated {}".into();
        source.text_digest = crate::canonical::hash_bytes(source.text.as_bytes());
        current
            .services
            .get_mut("orders")
            .unwrap()
            .sources
            .insert(source.id.clone(), source);
        let normalized = json!({"schema":"codeclew-java-compiler-fact/1.0","declarationKind":"CLASS","symbolIdentity":"class:orders.Unrelated","scope":":main"});
        add_observation(
            &mut current,
            Observation {
                id: "000-unrelated-declaration".into(),
                kind: "SYMBOL".into(),
                service: "orders".into(),
                symbol: "class:orders.Unrelated".into(),
                digest: digest(&normalized).unwrap(),
                normalized,
                source_ids: vec!["000-unrelated-source".into()],
            },
        );
        let replay = work::replay_process_work(&saved, &current).unwrap();
        assert_ne!(saved.handles, replay.handles);
        let before = projection(&saved).unwrap();
        let after = projection(&replay).unwrap();
        assert!(before == after, "{}", first_difference(&before, &after, ""));
        assert_eq!(
            compare_loaded(&saved, &current).unwrap()["status"],
            "CURRENT"
        );
    }

    #[test]
    fn new_revision_reuses_valid_changed_receipts_only_after_semantic_replay() {
        let temporary = tempfile::tempdir().unwrap();
        Repository::init(temporary.path(), "New revision replay fixture").unwrap();
        let repo = Repository::open(temporary.path()).unwrap();
        let mut saved = fixture();
        let mut inputs = repo.inputs().unwrap();
        let service: super::super::model::Service = serde_json::from_value(json!({
            "schema":"codeclew-documentation-service/1.0","id":"orders","title":"Orders",
            "repositoryId":"fixture-orders","repository":"https://example.invalid/orders",
            "language":"java","profile":"java-17plus-maven-read-only","compilations":[":/main"],"targetRef":"main"
        })).unwrap();
        saved
            .checked
            .services
            .get_mut("orders")
            .unwrap()
            .service_digest = digest(&service).unwrap();
        inputs.services.insert("orders".into(), service);
        saved.checked.input_digest = digest(&inputs).unwrap();
        saved.checked.source_inputs = Some(super::super::check::SourceInputs {
            schema: super::super::check::SOURCE_INPUTS_SCHEMA.into(),
            input_digest: saved.checked.input_digest.clone(),
            inputs,
            selected_services: std::collections::BTreeSet::from(["orders".into()]),
            retained_services: std::collections::BTreeSet::new(),
        });
        for source in saved
            .checked
            .services
            .get_mut("orders")
            .unwrap()
            .sources
            .values_mut()
        {
            source.occurrence = Some(super::super::model::SourceOccurrence {
                snapshot: "old-immutable-source-snapshot".into(),
                blob: source.text_digest.clone(),
                start_byte: 0,
                end_byte: source.text.len(),
            });
        }
        let source = saved.checked.services["orders"].sources["endpoint-source"].clone();
        let site = json!({"sourceId":source.id,"service":source.service,"revision":source.revision,"file":source.file,
            "sourceDigest":source.text_digest,"evidenceDigest":source.evidence_digest,"sourceStatus":"SOURCE_RETAINED",
            "sourceByteStart":0,"sourceByteEnd":source.text.len(),"byteStart":0,"byteEnd":source.text.len(),"spanDigest":source.text_digest});
        let mut declaration = saved.checked.dependencies["endpoint-declaration"].clone();
        declaration.normalized["declarationSite"] = site.clone();
        declaration.digest = digest(&declaration.normalized).unwrap();
        add_observation(&mut saved.checked, declaration);
        let mut flow = saved.checked.dependencies["flow-endpoint-service"].clone();
        flow.normalized["schema"] = json!("codeclew-java-compiler-fact/1.0");
        flow.normalized["callSite"] = site;
        flow.digest = digest(&flow.normalized).unwrap();
        add_observation(&mut saved.checked, flow);
        let parameter = source.text.find("request)").unwrap();
        let root = saved.checked.dependencies["endpoint-declaration"]
            .symbol
            .clone();
        let normalized = json!({"schema":"codeclew-java-compiler-fact/1.0","resolution":"COMPILER_EXACT",
            "variableIdentity":"parameter:orders.Controller#handle:request","variableKind":"PARAMETER","scope":":main",
            "callableObservationId":"endpoint-declaration","declarationObservationId":"request-variable-declaration",
            "variableSite":{"sourceId":source.id,"service":source.service,"revision":source.revision,"file":source.file,
                "sourceDigest":source.text_digest,"evidenceDigest":source.evidence_digest,"sourceStatus":"SOURCE_RETAINED",
                "sourceByteStart":0,"sourceByteEnd":source.text.len(),"byteStart":parameter,"byteEnd":parameter+7,
                "spanDigest":crate::canonical::hash_bytes(&source.text.as_bytes()[parameter..parameter+7])}});
        add_observation(
            &mut saved.checked,
            Observation {
                id: "request-variable-declaration".into(),
                kind: "VARIABLE_DECLARATION".into(),
                service: "orders".into(),
                symbol: root,
                digest: digest(&normalized).unwrap(),
                normalized,
                source_ids: vec![source.id],
            },
        );
        saved = work::replay_process_work(&saved, &saved.checked).unwrap();
        let (_, audit) = operation_packet::build(&saved).unwrap();
        let mut current = saved.checked.clone();
        let evidence = current.services.get_mut("orders").unwrap();
        evidence.revision = "new-committed-revision".into();
        for source in evidence.sources.values_mut() {
            source.revision = evidence.revision.clone();
            source.evidence_digest = digest(&(source.id.as_str(), "new-source-receipt")).unwrap();
            source.occurrence.as_mut().unwrap().snapshot = "new-immutable-source-snapshot".into();
        }
        let mut changed_bindings = 0;
        let ids: Vec<_> = current.dependencies.keys().cloned().collect();
        for id in ids {
            let mut observation = current.dependencies[&id].clone();
            let mut changed = false;
            for key in ["variableSite", "callSite", "declarationSite"] {
                if let Some(site) = observation
                    .normalized
                    .get_mut(key)
                    .and_then(Value::as_object_mut)
                {
                    let source =
                        &current.services["orders"].sources[site["sourceId"].as_str().unwrap()];
                    site.insert("revision".into(), json!(source.revision));
                    site.insert("evidenceDigest".into(), json!(source.evidence_digest));
                    changed = true;
                    changed_bindings += 1;
                }
            }
            if changed {
                observation.digest = digest(&observation.normalized).unwrap();
                add_observation(&mut current, observation);
            }
        }
        assert_eq!(changed_bindings, 3);
        let source_inputs = current.source_inputs.as_mut().unwrap();
        source_inputs.inputs.manifest.title = "Unrelated declaration change at new revision".into();
        source_inputs.input_digest = digest(&source_inputs.inputs).unwrap();
        current.input_digest = source_inputs.input_digest.clone();
        current.context_digest = digest(&current.dependencies).unwrap();
        assert_ne!(saved.checked.input_digest, current.input_digest);
        assert_ne!(saved.checked.context_digest, current.context_digest);
        let strict = super::super::answer_context::compare(&saved, &audit, &current).unwrap();
        assert_eq!(strict["status"], "STALE");
        assert_eq!(strict["changedPins"].as_array().unwrap().len(), 3);
        assert!(strict["changedSources"].as_array().unwrap().is_empty());
        assert!(
            strict["missingPinsOrBindings"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        let replay = work::replay_process_work(&saved, &current).unwrap();
        let before = projection(&saved).unwrap();
        let after = projection(&replay).unwrap();
        assert!(before == after, "{}", first_difference(&before, &after, ""));
        let compared = compare(&repo, &saved, &current).unwrap();
        assert_eq!(compared["status"], "CURRENT");
        let production = super::super::answer_context::compare_with_verified_replay(
            &saved, &audit, &current, &compared,
        )
        .unwrap();
        assert_eq!(production["status"], "CURRENT");
        assert_eq!(
            production["strictDependencyDigestChanges"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        assert!(production["changedPins"].as_array().unwrap().is_empty());
        // A semantic declaration delta does not inherit the provenance-only
        // exception even when the same valid receipt changes also remain.
        let mut semantic = current.clone();
        let mut declaration = semantic.dependencies["endpoint-declaration"].clone();
        declaration.normalized["semanticDelta"] = json!("changed declaration policy");
        declaration.digest = digest(&declaration.normalized).unwrap();
        add_observation(&mut semantic, declaration);
        let semantic_replay = compare(&repo, &saved, &semantic).unwrap();
        assert_eq!(semantic_replay["status"], "STALE");
        let production_semantic = super::super::answer_context::compare_with_verified_replay(
            &saved,
            &audit,
            &semantic,
            &semantic_replay,
        )
        .unwrap();
        assert_ne!(production_semantic["status"], "CURRENT");
        assert!(
            production_semantic
                .get("strictDependencyDigestChanges")
                .is_none()
        );
        assert!(selected_provenance_only(&strict, &compared));
        assert!(!selected_provenance_only(
            &strict,
            &json!({"status":"STALE"})
        ));
        let mut unknown = strict.clone();
        unknown["missingPinsOrBindings"] = json!([{"reason":"SOURCE_BINDING_INVALID"}]);
        assert!(!selected_provenance_only(&unknown, &compared));
        let mut content = strict.clone();
        content["changedSources"] = json!([{"id":"endpoint-source"}]);
        assert!(!selected_provenance_only(&content, &compared));
        assert!(!repo.path(".codeclew/work").unwrap().exists());
    }

    #[test]
    fn replay_refuses_changed_guard_and_new_matching_callee() {
        let saved = fixture();
        let mut current = saved.checked.clone();
        let source = current
            .services
            .get_mut("orders")
            .unwrap()
            .sources
            .get_mut("endpoint-source")
            .unwrap();
        source.text = source.text.replace("request == null", "request != null");
        source.text_digest = crate::canonical::hash_bytes(source.text.as_bytes());
        assert_eq!(compare_loaded(&saved, &current).unwrap()["status"], "STALE");

        // The old selection sees no target. Adding a matching target must replay
        // its formerly negative lookup, even though every old record survived.
        let mut negative = saved.clone();
        let target = negative.checked.dependencies["flow-endpoint-service"].normalized["target"]
            .as_str()
            .unwrap()
            .to_owned();
        let ids: Vec<_> = negative
            .checked
            .dependencies
            .values()
            .filter(|record| record.kind == "SYMBOL" && record.symbol == target)
            .map(|record| record.id.clone())
            .collect();
        assert!(!ids.is_empty());
        let target_declaration = negative.checked.dependencies[&ids[0]].clone();
        for id in ids {
            negative.checked.dependencies.remove(&id);
            negative
                .checked
                .services
                .get_mut("orders")
                .unwrap()
                .observations
                .remove(&id);
        }
        negative = work::replay_process_work(&negative, &negative.checked).unwrap();
        let mut current = negative.checked.clone();
        add_observation(&mut current, target_declaration);
        assert_eq!(
            compare_loaded(&negative, &current).unwrap()["status"],
            "STALE"
        );
    }

    #[test]
    fn notes_membership_probe_refuses_new_tree_without_creating_work() {
        let temporary = tempfile::tempdir().unwrap();
        Repository::init(temporary.path(), "Replay fixture").unwrap();
        let repo = Repository::open(temporary.path()).unwrap();
        let saved = fixture();
        assert_eq!(
            compare(&repo, &saved, &saved.checked).unwrap()["status"],
            "CURRENT"
        );
        std::fs::create_dir(repo.path("notes").unwrap()).unwrap();
        std::fs::write(
            repo.path("notes/new.md").unwrap(),
            "New protected human context",
        )
        .unwrap();
        let compared = compare(&repo, &saved, &saved.checked).unwrap();
        assert_eq!(compared["status"], "STALE");
        assert_eq!(compared["reasons"], json!(["NOTES_MEMBERSHIP_CHANGED"]));
        assert!(!repo.path(".codeclew/work").unwrap().exists());
    }

    #[test]
    fn semantic_text_is_not_rewritten_as_a_local_handle() {
        let references = BTreeMap::from([("d1".into(), identity("DEPENDENCY", "method"))]);
        let mut value = json!({"text":"d1","name":"d1","reference":"d1","normalized":{"reference":"d1","evidence":["s1"],"nested":{"textDigest":"x","evidenceDigest":"y","text":"d1","service":"s1"}}});
        let semantic = value["normalized"].clone();
        normalize(&mut value, "", &references, &BTreeMap::new()).unwrap();
        assert_eq!(value["text"], "d1");
        assert_eq!(value["name"], "d1");
        assert_eq!(value["reference"], identity("DEPENDENCY", "method"));
        assert_eq!(value["normalized"], semantic);
    }

    #[test]
    fn corrupt_selected_source_and_unsupported_context_are_explicit() {
        let temporary = tempfile::tempdir().unwrap();
        Repository::init(temporary.path(), "Replay fixture").unwrap();
        let repo = Repository::open(temporary.path()).unwrap();
        let mut saved = fixture();
        saved
            .checked
            .services
            .get_mut("orders")
            .unwrap()
            .sources
            .get_mut("endpoint-source")
            .unwrap()
            .text_digest = "forged".into();
        let error = compare(&repo, &saved, &saved.checked).unwrap_err();
        assert!(error.message.contains("CORRUPT"));
        let mut unsupported = fixture();
        unsupported.request.source_data_context = false;
        assert!(
            compare(&repo, &unsupported, &unsupported.checked)
                .unwrap_err()
                .message
                .starts_with("ANSWER_REUSE_UNSUPPORTED:")
        );
    }

    #[test]
    fn portable_expectation_or_producer_rules_refuse_native_replay() {
        let temporary = tempfile::tempdir().unwrap();
        Repository::init(temporary.path(), "Producer boundary fixture").unwrap();
        let repo = Repository::open(temporary.path()).unwrap();
        let saved = fixture();
        let mut expected = saved.checked.clone();
        let policy = super::super::evidence_package::Expectation {
            schema: "codeclew-documentation-evidence-expectation/1.0".into(),
            service: "orders".into(),
            repository_id: "fixture-orders".into(),
            service_digest: expected.services["orders"].service_digest.clone(),
            revision: expected.services["orders"].revision.clone(),
            manifest_digest: digest(&"portable-manifest").unwrap(),
            sequence: 1,
        };
        expected.source_inputs = Some(super::super::check::SourceInputs {
            schema: super::super::check::SOURCE_INPUTS_SCHEMA.into(),
            input_digest: expected.input_digest.clone(),
            inputs: repo.inputs().unwrap(),
            selected_services: std::collections::BTreeSet::from(["orders".into()]),
            retained_services: std::collections::BTreeSet::new(),
        });
        expected
            .source_inputs
            .as_mut()
            .unwrap()
            .inputs
            .evidence_expectations
            .insert("orders".into(), policy);
        assert!(
            compare(&repo, &saved, &expected)
                .unwrap_err()
                .message
                .starts_with("ANSWER_REUSE_UNSUPPORTED:")
        );
        let mut prior = saved.clone();
        prior.checked = expected.clone();
        assert!(
            compare(&repo, &prior, &saved.checked)
                .unwrap_err()
                .message
                .starts_with("ANSWER_REUSE_UNSUPPORTED:")
        );
        let normalized = json!({"schema":"codeclew-documentation-portable-influence/1.0","expectation":{"sequence":1},"producerRules":"old-producer-rules"});
        let mut packaged = saved.clone();
        add_observation(
            &mut packaged.checked,
            Observation {
                id: "portable-package".into(),
                kind: "EVIDENCE_PACKAGE".into(),
                service: "orders".into(),
                symbol: "coordinator-selected-revision".into(),
                digest: digest(&normalized).unwrap(),
                normalized,
                source_ids: vec![],
            },
        );
        let mut changed_rules = packaged.checked.clone();
        let mut package = changed_rules.dependencies["portable-package"].clone();
        package.normalized["producerRules"] = json!("new-producer-rules");
        package.normalized["expectation"]["sequence"] = json!(2);
        package.digest = digest(&package.normalized).unwrap();
        add_observation(&mut changed_rules, package);
        assert!(
            compare(&repo, &packaged, &changed_rules)
                .unwrap_err()
                .message
                .starts_with("ANSWER_REUSE_UNSUPPORTED:")
        );
        assert!(
            compare(&repo, &saved, &changed_rules)
                .unwrap_err()
                .message
                .starts_with("ANSWER_REUSE_UNSUPPORTED:")
        );
        assert_eq!(
            packaged.checked.dependencies["endpoint-declaration"],
            changed_rules.dependencies["endpoint-declaration"]
        );
        assert!(!repo.path(".codeclew/work").unwrap().exists());
    }
}
