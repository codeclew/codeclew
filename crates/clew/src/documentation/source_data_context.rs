//! Bounded source-syntax data context over an immutable Work Check only.
use super::{
    digest, invalid,
    work::{Request, Work},
};
use crate::error::ClewError;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub(super) const MAX_PACKET_BYTES: usize = 65_536;
pub(super) struct Projection {
    pub context: Value,
    pub rows: Vec<Value>,
    pub citations: BTreeMap<String, String>,
}

pub(super) fn validate_request(request: &Request) -> Result<(), ClewError> {
    if request.source_data_context && request.context_profile.as_deref() != Some("process-graph-v1")
    {
        return Err(invalid("sourceDataContext requires process-graph-v1"));
    }
    Ok(())
}
fn reference(work: &Work, kind: &str, id: &str) -> Result<String, ClewError> {
    let mut found = work
        .handles
        .iter()
        .filter(|(_, h)| h.kind == kind && h.id == id);
    let result = found.next().ok_or_else(|| {
        invalid(format!(
            "sourceDataContext missing exact {kind} Work reference: {id}"
        ))
    })?;
    if found.next().is_some() {
        return Err(invalid("sourceDataContext ambiguous Work reference"));
    }
    Ok(result.0.clone())
}

/// Lossless, node-local interning of typed storage and condition/completion sets.
/// IDs refer only to these owned tables, never compiler or citation identities.
fn compact_state(state: &mut Value) -> Result<(), ClewError> {
    fn visit(
        value: &mut Value,
        tables: &mut BTreeMap<String, BTreeMap<String, Value>>,
        keys: &mut BTreeMap<String, BTreeMap<String, String>>,
    ) -> Result<(), ClewError> {
        match value {
            Value::Object(object) => {
                for (name, item) in object.iter_mut() {
                    let table = match name.as_str() {
                        "storage" if item.is_object() => Some(("storages", "storageRef")),
                        "conditions" if item.as_array().is_some_and(|a| !a.is_empty()) => {
                            Some(("guardSets", "guardSetRef"))
                        }
                        "normalCompletionOf" if item.as_array().is_some_and(|a| !a.is_empty()) => {
                            Some(("completionSets", "completionSetRef"))
                        }
                        _ => None,
                    };
                    if let Some((table, reference)) = table {
                        let canonical = serde_json::to_string(item).map_err(super::io_error)?;
                        let ids = keys.entry(table.into()).or_default();
                        let next = format!("r{}", ids.len() + 1);
                        let id = ids.entry(canonical).or_insert(next).clone();
                        tables
                            .entry(table.into())
                            .or_default()
                            .entry(id.clone())
                            .or_insert_with(|| item.clone());
                        *item = Value::Object(serde_json::Map::from_iter([(
                            reference.to_owned(),
                            json!(id),
                        )]));
                    } else {
                        visit(item, tables, keys)?;
                    }
                }
            }
            Value::Array(items) => {
                for item in items {
                    visit(item, tables, keys)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    let mut tables = BTreeMap::new();
    let mut keys = BTreeMap::new();
    visit(state, &mut tables, &mut keys)?;
    state["encoding"] = json!("SHARED_STORAGE_GUARD_COMPLETION_SETS");
    state["shared"] = json!(tables);
    Ok(())
}

pub(super) fn build(work: &Work) -> Result<Option<Projection>, ClewError> {
    validate_request(&work.request)?;
    if !work.request.source_data_context {
        return Ok(None);
    }
    if work.snapshot.as_deref().is_none_or(str::is_empty) {
        return Err(invalid(
            "sourceDataContext requires immutable Work snapshot",
        ));
    }
    let root =
        super::process_graph::resolve_work_root(&work.subject, &work.request, &work.checked)?;
    let graph =
        super::static_pages::source_data_graph(&work.checked, &root.service, &root.declaration.id)?;
    let mut retained_bytes = 0usize;
    let mut counted_sources = BTreeSet::new();
    for node in graph.nodes.values() {
        for source in node.sources.values() {
            if counted_sources.insert(source.id.clone()) {
                retained_bytes = retained_bytes.saturating_add(source.text.len());
                if retained_bytes > MAX_PACKET_BYTES {
                    return Err(invalid(
                        "sourceDataContext exceeds 65536-byte complete-source addition; narrow the root",
                    ));
                }
            }
        }
    }
    let mut rows = BTreeMap::new();
    let mut sources = BTreeMap::new();
    let mut nodes = Vec::new();
    let mut bindings = BTreeMap::new();
    for node in graph.nodes.values() {
        let mut labels = BTreeMap::new();
        let state = node
            .data_state
            .as_ref()
            .ok_or_else(|| invalid("sourceDataContext has no data projection"))?;
        let mut observations = node.observations.clone();
        let mut retained_sources = node.sources.clone();
        for id in state
            .field_declarations
            .iter()
            .chain(&state.property_declarations)
        {
            let observation = work.checked.dependencies.get(id).ok_or_else(|| {
                invalid("sourceDataContext examined member declaration is unavailable")
            })?;
            observations.insert(id.clone(), observation.clone());
            for source_id in &observation.source_ids {
                let source = work
                    .checked
                    .services
                    .get(&observation.service)
                    .and_then(|s| s.sources.get(source_id))
                    .ok_or_else(|| {
                        invalid("sourceDataContext examined member source is unavailable")
                    })?;
                if counted_sources.insert(source_id.clone()) {
                    retained_bytes = retained_bytes.saturating_add(source.text.len());
                    if retained_bytes > MAX_PACKET_BYTES {
                        return Err(invalid(
                            "sourceDataContext exceeds 65536-byte complete-source addition; narrow the root",
                        ));
                    }
                }
                retained_sources.insert(source_id.clone(), source.clone());
            }
        }
        for (id, record) in &observations {
            let exact = work.checked.dependencies.get(id).ok_or_else(|| {
                invalid("sourceDataContext dependency is absent from immutable Check")
            })?;
            if exact != record || work.influence.get(id) != Some(&record.digest) {
                return Err(invalid(
                    "sourceDataContext dependency provenance differs from immutable Work",
                ));
            }
            let label = reference(work, "DEPENDENCY", id)?;
            rows.insert(
                ("DEPENDENCY", id.clone()),
                json!({"kind":"DEPENDENCY","id":id,"record":exact}),
            );
            if !matches!(
                record.kind.as_str(),
                "VARIABLE_ACCESS" | "VARIABLE_DECLARATION"
            ) {
                labels.insert(id.clone(), label);
            }
        }
        for (id, source) in &retained_sources {
            let exact = work
                .checked
                .services
                .get(&source.service)
                .and_then(|s| s.sources.get(id))
                .ok_or_else(|| {
                    invalid("sourceDataContext source is absent from immutable Check")
                })?;
            if exact != source {
                return Err(invalid(
                    "sourceDataContext source provenance differs from immutable Check",
                ));
            }
            let label = reference(work, "SOURCE", id)?;
            rows.insert(
                ("SOURCE", id.clone()),
                json!({"kind":"SOURCE","id":id,"record":exact}),
            );
            sources.insert(label.clone(),json!({"reference":label,"authority":source.authority,"text":source.text,"evidence":[label]}));
        }
        let mut used_spans: BTreeSet<_> = state
            .definitions
            .iter()
            .filter_map(|d| d.citation_id.as_deref())
            .chain(state.gaps.iter().filter_map(|g| g.citation_id.as_deref()))
            .collect();
        used_spans.extend(
            node.calls
                .iter()
                .filter_map(|edge| edge.call.as_ref().map(|call| call.citation_id.as_str())),
        );
        if let Some(citation) = node.callable.citation_id.as_deref() {
            used_spans.insert(citation);
        }
        for (id, citation) in node
            .citations
            .iter()
            .filter(|(id, _)| used_spans.contains(id.as_str()))
        {
            let source = node.sources.get(&citation.source_id).ok_or_else(|| {
                invalid("sourceDataContext citation has no exact retained source")
            })?;
            let text = source
                .text
                .get(citation.start_byte..citation.end_byte)
                .ok_or_else(|| invalid("sourceDataContext citation range is invalid UTF-8"))?;
            if crate::canonical::hash_bytes(text.as_bytes()) != citation.text_digest
                || source.service != citation.service
                || source.revision != citation.revision
                || source.file != citation.file
                || source.evidence_digest != citation.evidence_digest
            {
                return Err(invalid(
                    "sourceDataContext citation provenance differs from retained source",
                ));
            }
            let label = reference(work, "SOURCE", &source.id)?;
            let binding = json!({"reference":label,"sourceDigest":digest(source)?,"startByte":citation.start_byte,"endByte":citation.end_byte,"textDigest":citation.text_digest});
            if let Some(old) = bindings.insert(id.clone(), binding.clone())
                && old != binding
            {
                return Err(invalid("sourceDataContext conflicting citation identities"));
            }
        }
        let mut state = serde_json::to_value(
            node.data_state
                .as_ref()
                .ok_or_else(|| invalid("sourceDataContext has no bounded data projection"))?,
        )
        .map_err(super::io_error)?;
        for collection in ["definitions", "gaps"] {
            for item in state[collection].as_array_mut().unwrap() {
                if let Some(span) = item["citationId"].as_str().map(str::to_owned) {
                    let label = bindings
                        .get(&span)
                        .and_then(|b| b["reference"].as_str())
                        .ok_or_else(|| invalid("sourceDataContext unknown retained span"))?;
                    item["sourceSpan"] = json!(span);
                    item["citationId"] = json!(label);
                }
            }
        }
        state["fieldDeclarations"] = json!(
            node.data_state
                .as_ref()
                .unwrap()
                .field_declarations
                .iter()
                .map(|id| reference(work, "DEPENDENCY", id))
                .collect::<Result<Vec<_>, _>>()?
        );
        if !node
            .data_state
            .as_ref()
            .unwrap()
            .property_declarations
            .is_empty()
        {
            state["propertyDeclarations"] = json!(
                node.data_state
                    .as_ref()
                    .unwrap()
                    .property_declarations
                    .iter()
                    .map(|id| reference(work, "DEPENDENCY", id))
                    .collect::<Result<Vec<_>, _>>()?
            );
        }
        let call_sites: Vec<_> = node
            .calls
            .iter()
            .filter_map(|call| call.call.as_ref().map(|projection| (call, projection)))
            .map(|(call, projection)| {
                let span = &projection.citation_id;
                Ok(json!({"occurrence":call.occurrence_path,"sourceSpan":span,"citationId":bindings.get(span).and_then(|b|b["reference"].as_str()).ok_or_else(||invalid("sourceDataContext unknown call span"))?,"status":call.status,"targetNode":call.target_node,"targetAuthority":"DECLARED_TARGET_SOURCE_CONDITIONAL"}))
            })
            .collect::<Result<_, ClewError>>()?;
        compact_state(&mut state)?;
        let variable_bindings: Vec<_> = observations
            .values()
            .filter(|o| matches!(o.kind.as_str(), "VARIABLE_ACCESS" | "VARIABLE_DECLARATION"))
            .map(|o| (&o.id, &o.digest))
            .collect();
        nodes.push(json!({"id":node.id,"service":node.service,"scope":node.scope,"symbol":node.callable.symbol,"declarationReference":reference(work,"DEPENDENCY",&node.callable.declaration_id)?,"examinedSourceDigest":node.examined_source_digest,"dataState":state,"callSites":call_sites,"variableFactCount":variable_bindings.len(),"variableFactsDigest":digest(&variable_bindings)?,"evidence":labels.values().collect::<Vec<_>>() }));
    }
    let data_digests: Vec<_> = graph
        .nodes
        .values()
        .map(|n| (&n.id, n.data_state.as_ref().map(|s| &s.data_state_digest)))
        .collect();
    let examined: Vec<_> = graph
        .nodes
        .values()
        .map(|n| (&n.id, &n.examined_source_digest))
        .collect();
    let sources: Vec<_> = sources.into_values().collect();
    let context = json!({"schema":"codeclew-source-data-context/1.0","authority":"SOURCE_SYNTAX_WITH_COMPILER_VARIABLE_IDENTITY","meaningReview":"UNASSESSED","runtimeStatus":"UNKNOWN","snapshot":work.snapshot,"rootDeclarationReference":reference(work,"DEPENDENCY",&root.declaration.id)?,"sourceDataDigest":digest(&data_digests)?,"examinedSourceDigest":digest(&examined)?,"bounds":{"maxDepth":graph.max_depth,"maxAdditionalBodies":graph.max_additional_bodies,"maxAdditionalSourceBytes":graph.max_additional_source_bytes,"maxPacketAdditionBytes":MAX_PACKET_BYTES},"nodes":nodes,"sourceSpans":bindings,"sources":sources,"limitations":["Guarded definitions are source-syntax alternatives, not runtime values or compiler dataflow proof.","Declaration identity does not establish receiver instance identity; interference and opaque frontiers remain unresolved.","Actual/formal and return mappings are DECLARED_TARGET_SOURCE_CONDITIONAL; normalCompletionOf is a source-order condition, not observed completion.","No delivery, incident, queue, callback, runtime dispatch, or current source freshness is established."]});
    let mut citations = BTreeMap::new();
    for row in rows.values() {
        if matches!(
            row["record"]["kind"].as_str(),
            Some("VARIABLE_ACCESS" | "VARIABLE_DECLARATION")
        ) {
            continue;
        }
        citations.insert(
            reference(
                work,
                row["kind"].as_str().unwrap(),
                row["id"].as_str().unwrap(),
            )?,
            "Retained source data".to_owned(),
        );
    }
    // Count the entire owned context, complete sources, property names and ALL
    // potential extra citation entries, even labels already in the base packet.
    // This conservative envelope bounds the actual packet addition after merge.
    if super::bytes(&json!({"sourceDataContext":context,"citations":citations}))?.len()
        > MAX_PACKET_BYTES
    {
        return Err(invalid(
            "sourceDataContext exceeds 65536-byte packet addition; select a smaller callable root",
        ));
    }
    Ok(Some(Projection {
        context,
        rows: rows.into_values().collect(),
        citations,
    }))
}

pub(super) fn validate_saved(work: &Work, packet: &Value) -> Result<(), ClewError> {
    let expected = build(work)?;
    if packet.get("sourceDataContext") != expected.as_ref().map(|p| &p.context) {
        return Err(invalid(
            "saved sourceDataContext differs from immutable Work projection",
        ));
    }
    if let Some(projection) = expected {
        for label in projection.citations.keys() {
            if packet["citations"][label]
                .as_str()
                .is_none_or(|text| text.is_empty())
            {
                return Err(invalid(
                    "saved sourceDataContext citation delivery is incomplete",
                ));
            }
        }
    }
    Ok(())
}

pub(super) fn profile_rows(work: &Work) -> Result<Vec<Value>, ClewError> {
    let mut rows = super::endpoint_context::process_profile_rows(work)?;
    if let Some(projection) = build(work)? {
        let mut keys: BTreeSet<_> = rows
            .iter()
            .map(|r| {
                (
                    r["kind"].as_str().unwrap_or_default().to_owned(),
                    r["id"].as_str().unwrap_or_default().to_owned(),
                )
            })
            .collect();
        for row in projection.rows {
            let key = (
                row["kind"].as_str().unwrap().to_owned(),
                row["id"].as_str().unwrap().to_owned(),
            );
            if keys.insert(key) {
                rows.push(row);
            }
        }
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::documentation::static_pages::model::NodeDataState;
    fn expanded(mut state: Value) -> Value {
        let tables = state.as_object_mut().unwrap().remove("shared").unwrap();
        state.as_object_mut().unwrap().remove("encoding");
        fn walk(value: &mut Value, tables: &Value) {
            if let Some(object) = value.as_object()
                && object.len() == 1
            {
                for (key, table) in [
                    ("storageRef", "storages"),
                    ("guardSetRef", "guardSets"),
                    ("completionSetRef", "completionSets"),
                ] {
                    if let Some(id) = object.get(key).and_then(Value::as_str) {
                        *value = tables[table][id].clone();
                        return;
                    }
                }
            }
            match value {
                Value::Object(object) => {
                    for item in object.values_mut() {
                        walk(item, tables);
                    }
                }
                Value::Array(items) => {
                    for item in items {
                        walk(item, tables);
                    }
                }
                _ => {}
            }
        }
        walk(&mut state, &tables);
        state
    }
    #[test]
    fn realistic_compiler_shaped_ir_is_losslessly_shared_within_packet_bound() {
        let storage = json!({"identity":"field:class:example.linked.ChildWorker#lastRequest:Ljava/lang/String;","kind":"FIELD","receiver":"THIS"});
        let guards = json!([{"expression":"task != null && task.name != null && !task.name.isEmpty()","holds":true},{"expression":"request == null || gateway == null","holds":false}]);
        let completions = json!([{"occurrence":"body/4/call/0","conditions":guards},{"occurrence":"body/8/call/0","conditions":guards}]);
        let definitions:Vec<_>=(0..96).map(|i|json!({"id":format!("definition-{i}"),"storage":storage,"value":if i==0 {json!({"kind":"INPUT","storage":storage})}else {json!({"kind":"READ","storage":storage,"alternatives":[format!("definition-{}",i-1)]})},"conditions":guards,"normalCompletionOf":completions,"citationId":"s1"})).collect();
        let original = json!({"schema":"codeclew-native-source-data-state/1.0","authority":"SOURCE_SYNTAX_WITH_COMPILER_VARIABLE_IDENTITY","dataStateDigest":"saved-native-digest","definitions":definitions,"calls":[],"fieldDeclarations":["field-declaration"],"gaps":[{"code":"CALL_FIELD_INTERFERENCE","detail":"Runtime field value remains opaque","citationId":"s1"}]});
        let typed: NodeDataState = serde_json::from_value(original.clone()).unwrap();
        let before = super::super::bytes(&original).unwrap().len();
        let mut compact = original.clone();
        compact_state(&mut compact).unwrap();
        let after = super::super::bytes(&compact).unwrap().len();
        assert!(before > MAX_PACKET_BYTES, "{before}");
        assert!(after < MAX_PACKET_BYTES / 2, "{after}");
        let restored = expanded(compact.clone());
        assert_eq!(restored, original);
        assert_eq!(
            serde_json::from_value::<NodeDataState>(restored).unwrap(),
            typed
        );
        assert_eq!(compact["definitions"].as_array().unwrap().len(), 96);
        assert_eq!(compact["shared"]["storages"].as_object().unwrap().len(), 1);
        assert_eq!(compact["shared"]["guardSets"].as_object().unwrap().len(), 1);
        assert_eq!(
            compact["shared"]["completionSets"]
                .as_object()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            compact["definitions"][95]["value"]["alternatives"],
            json!(["definition-94"])
        );
        assert_eq!(compact["dataStateDigest"], original["dataStateDigest"]);
    }
}
