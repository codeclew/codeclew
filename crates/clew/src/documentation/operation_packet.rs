//! Compact author-facing projection of one immutable endpoint Work profile.
//!
//! This module deliberately does not resolve source, page through Work, or
//! record read receipts. The full selected rows are retained only in the
//! optional audit value returned alongside the compact packet.

use super::{digest, endpoint_context, invalid, source_steps, work::Work};
use crate::error::ClewError;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub const PACKET_SCHEMA: &str = "codeclew-documentation-reader-packet/1.0";
pub const AUDIT_SCHEMA: &str = "codeclew-documentation-reader-packet-audit/1.0";

/// Build the short author packet and a separate audit projection of every
/// immutable profile row selected for it.
pub(super) fn build(work: &Work) -> Result<(Value, Value), ClewError> {
    if work.request.context_profile.as_deref() != Some(endpoint_context::PROFILE) {
        return Err(invalid(
            "READER_PACKET_PROFILE_REQUIRED: prepare fresh Work with endpoint-context-v3",
        ));
    }

    let rows = endpoint_context::profile_rows(work)?;
    let mut row_counts = BTreeMap::<String, usize>::new();
    let mut selected_by_key = BTreeMap::<(String, String), (String, Value)>::new();
    let mut selected_by_reference = BTreeMap::<String, (String, Value)>::new();
    let mut records = Vec::with_capacity(rows.len());
    let mut selected_bindings = Vec::with_capacity(rows.len());
    let mut used_labels = BTreeSet::new();

    for row in rows {
        let kind = row["kind"]
            .as_str()
            .ok_or_else(|| invalid("reader packet selected row has no kind"))?
            .to_owned();
        let id = row["id"]
            .as_str()
            .ok_or_else(|| invalid("reader packet selected row has no id"))?
            .to_owned();
        let work_reference = work
            .handles
            .iter()
            .find(|(_, handle)| handle.kind == kind && handle.id == id)
            .map(|(reference, _)| reference.clone());
        let label = work_reference
            .clone()
            .unwrap_or_else(|| synthetic_label(&kind, &mut row_counts));
        if !used_labels.insert(label.clone()) {
            return Err(invalid("reader packet evidence labels are ambiguous"));
        }
        let record_digest = digest(&row)?;
        selected_bindings.push(json!({"label":label,"recordDigest":record_digest}));
        records.push(json!({
            "label":label,
            "kind":kind,
            "id":id,
            "workReference":work_reference.clone(),
            "recordDigest":record_digest,
            "deliveredToAuthor":false,
            "row":row.clone()
        }));
        selected_by_key.insert((kind.clone(), id.clone()), (label.clone(), row.clone()));
        if let Some(reference) = work_reference {
            selected_by_reference.insert(reference, (label, row));
        }
    }

    let packet_row = rows_by_kind(&selected_by_key, "ENDPOINT_CONTEXT_PACKET")
        .ok_or_else(|| invalid("endpoint-context-v3 selected no packet row"))?;
    let packet_record = &packet_row.1["record"];
    let packet_label = packet_row.0;
    let coverage_row = rows_by_kind(&selected_by_key, "COVERAGE")
        .ok_or_else(|| invalid("endpoint-context-v3 selected no coverage row"))?;
    let coverage_label = coverage_row.0;
    let coverage_record = &coverage_row.1["record"];
    let mut citations = BTreeMap::<String, String>::new();
    cite(&mut citations, packet_label, "selected endpoint context");
    cite(&mut citations, coverage_label, "saved capture coverage");

    let entrypoint_id = work.request.entrypoint.as_deref().unwrap_or_default();
    let entrypoint = selected_by_key.get(&("ENTRYPOINT".to_owned(), entrypoint_id.to_owned()));
    let endpoint_evidence = if let Some((label, _)) = entrypoint {
        cite(&mut citations, label, "HTTP endpoint declaration");
        vec![label.clone()]
    } else {
        vec![packet_label.clone()]
    };
    let endpoint_record = entrypoint.map(|(_, row)| &row["record"]);

    let mut type_groups = Vec::new();
    for group in packet_record["inputAndOutputTypes"]
        .as_array()
        .into_iter()
        .flatten()
    {
        let mut fields = Vec::new();
        for reference in group["fieldReferences"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            let (label, row) = selected_by_reference
                .get(reference)
                .ok_or_else(|| invalid("reader packet DTO field reference is not selected"))?;
            let normalized = &row["record"]["normalized"];
            cite(&mut citations, label, "captured DTO field declaration");
            fields.push(json!({
                "name":normalized["name"],
                "typeDescriptor":retained_declared_type_descriptor(normalized),
                "modifiers":array_or_empty(&normalized["modifiers"]),
                "annotations":array_or_empty(&normalized["annotations"]),
                "sourceTokens":array_or_empty(&normalized["sourceTokens"]),
                "evidence":[label]
            }));
        }
        fields.sort_by(|left, right| left["name"].as_str().cmp(&right["name"].as_str()));
        type_groups.push(json!({
            "identity":group["identity"],
            "directions":group["directions"],
            "fields":fields,
            "evidence":[packet_label]
        }));
    }
    type_groups.sort_by(|left, right| left["identity"].as_str().cmp(&right["identity"].as_str()));

    let raw_nodes = packet_record["callGraph"]["nodes"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut node_ids = BTreeMap::<String, String>::new();
    let mut call_nodes = Vec::with_capacity(raw_nodes.len());
    let mut method_sources = Vec::<Value>::new();
    let mut emitted_method_sources = BTreeSet::<String>::new();
    let mut method_bodies = Vec::<Value>::new();
    let mut body_ranges = BTreeMap::<(String, usize, usize), String>::new();
    let mut node_sources = BTreeMap::<String, String>::new();

    for raw_node in &raw_nodes {
        let node_id = raw_node["id"].as_str().unwrap_or_default().to_owned();
        let identity = raw_node["symbolIdentity"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let scope = raw_node["scope"].as_str().unwrap_or_default().to_owned();
        node_ids.insert(identity.clone(), node_id.clone());
        let mut body_id = None;
        let mut node_evidence = vec![packet_label.clone()];
        if let Some(source_reference) = raw_node["bodyReference"].as_str()
            && let Some((source_label, source_row)) = selected_by_reference.get(source_reference)
        {
            let source = &source_row["record"];
            if emitted_method_sources.insert(source_reference.to_owned()) {
                method_sources.push(json!({
                    "reference":source_reference,
                    "sourceAuthority":source["authority"],
                    "text":source["text"],
                    "evidence":[source_label]
                }));
                cite(
                    &mut citations,
                    source_label,
                    "retained source for selected method bodies",
                );
            }
            if let Some(text) = source["text"].as_str()
                && let Some((start, end)) = source_steps::method_body(text, &identity)
            {
                let key = (source_reference.to_owned(), start, end);
                let id = if let Some(id) = body_ranges.get(&key) {
                    id.clone()
                } else {
                    let id = format!("b{}", method_bodies.len() + 1);
                    body_ranges.insert(key, id.clone());
                    method_bodies.push(json!({
                        "id":id,
                        "nodes":[node_id],
                        "source":source_reference,
                        "startByte":start,
                        "endByte":end,
                        "evidence":[packet_label,source_label]
                    }));
                    id
                };
                if let Some(body) = method_bodies.iter_mut().find(|body| body["id"] == id)
                    && !body["nodes"]
                        .as_array()
                        .is_some_and(|nodes| nodes.iter().any(|node| node == &node_id))
                {
                    body["nodes"]
                        .as_array_mut()
                        .expect("method body nodes are initialized")
                        .push(json!(node_id));
                }
                body_id = Some(id);
                node_sources.insert(node_id.clone(), source_reference.to_owned());
                node_evidence.push(source_reference.to_owned());
            }
        }
        call_nodes.push(json!({
            "id":node_id,
            "identity":identity,
            "scope":scope,
            "source":raw_node["bodyReference"],
            "bodyId":body_id,
            "evidence":node_evidence
        }));
    }

    let mut call_edges = Vec::<Value>::new();
    let mut provider_edges =
        BTreeMap::<(String, String, String, String, String), BTreeSet<String>>::new();
    for reference in packet_record["callGraph"]["providerEdgeFactReferences"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        let (label, row) = selected_by_reference
            .get(reference)
            .ok_or_else(|| invalid("reader packet call fact reference is not selected"))?;
        let record = &row["record"];
        let normalized = &record["normalized"];
        let source_identity = if record["kind"] == "CALL_RELATION" {
            normalized["sourceIdentity"].as_str()
        } else {
            record["symbol"].as_str()
        }
        .unwrap_or_default();
        let from = node_ids
            .get(source_identity)
            .ok_or_else(|| invalid("reader packet call fact source has no selected node"))?;
        let target_identity = if record["kind"] == "CALL_RELATION" {
            normalized["targetIdentity"].as_str()
        } else {
            normalized["target"].as_str()
        }
        .unwrap_or_default();
        if target_identity.is_empty() {
            return Err(invalid("reader packet call fact has no captured target"));
        }
        let kind = if record["kind"] == "CALL_RELATION" {
            normalized["relationKind"].as_str().unwrap_or("CALLS")
        } else {
            normalized["kind"].as_str().unwrap_or("CALL")
        };
        let authority = if record["kind"] == "CALL_RELATION"
            && normalized["relationKind"] == "REFERENCES"
            && normalized["resolution"] == "COMPILER_EXACT"
        {
            "COMPILER_EXACT_REFERENCE_RELATION"
        } else if record["kind"] == "CALL_RELATION" && normalized["resolution"] == "COMPILER_EXACT"
        {
            "COMPILER_EXACT_CALL_RELATION"
        } else if record["kind"] == "FLOW" {
            "RETAINED_FLOW_TARGET"
        } else {
            "UNVERIFIED_PROVIDER_RELATION"
        };
        let scope = normalized["scope"].as_str().unwrap_or_default();
        let edge_key = (
            from.clone(),
            target_identity.to_owned(),
            kind.to_owned(),
            scope.to_owned(),
            authority.to_owned(),
        );
        provider_edges
            .entry(edge_key)
            .or_default()
            .insert(label.clone());
        cite(
            &mut citations,
            label,
            if kind == "REFERENCES" {
                "retained provider reference evidence"
            } else {
                "retained provider call evidence"
            },
        );
    }
    for ((from, target_identity, kind, scope, authority), evidence) in provider_edges {
        let to_node = node_ids.get(&target_identity).cloned();
        call_edges.push(json!({
            "from":from,
            "toNode":to_node,
            "targetIdentity":if to_node.is_none(){Some(target_identity)}else{None},
            "kind":kind,
            "scope":scope,
            "authority":authority,
            "evidence":evidence.into_iter().collect::<Vec<_>>()
        }));
    }

    for candidate in packet_record["callGraph"]["sourceReferenceCandidates"]
        .as_array()
        .into_iter()
        .flatten()
    {
        let from = candidate["fromNode"]
            .as_str()
            .ok_or_else(|| invalid("reader packet source candidate has no caller node"))?;
        if !node_ids.values().any(|node| node == from) {
            return Err(invalid(
                "reader packet source candidate caller is not selected",
            ));
        }
        let to_node = candidate["toNode"].as_str().map(str::to_owned);
        let target_identity = candidate["targetIdentity"].as_str().map(str::to_owned);
        if to_node.is_none() && target_identity.is_none() {
            return Err(invalid("reader packet source candidate has no target"));
        }
        let mut evidence = vec![packet_label.clone()];
        if let Some(source_reference) = node_sources.get(from) {
            evidence.push(source_reference.clone());
            if let Some((label, _)) = selected_by_reference.get(source_reference) {
                cite(&mut citations, label, "retained source for call candidate");
            }
        }
        let mut receiver_field = None;
        if let Some(reference) = candidate["receiverFieldReference"].as_str() {
            let (label, row) = selected_by_reference
                .get(reference)
                .ok_or_else(|| invalid("reader packet receiver field reference is not selected"))?;
            let normalized = &row["record"]["normalized"];
            if row["record"]["kind"] != "SYMBOL" || normalized["declarationKind"] != "FIELD" {
                return Err(invalid(
                    "reader packet receiver evidence is not a FIELD declaration",
                ));
            }
            cite(
                &mut citations,
                label,
                "declared field type for source receiver candidate",
            );
            evidence.push(label.clone());
            receiver_field = Some(json!({
                "name":normalized["name"],
                "typeDescriptor":retained_declared_type_descriptor(normalized),
                "evidence":[label]
            }));
        }
        let mut call_edge = json!({
            "from":from,
            "toNode":to_node,
            "targetIdentity":target_identity,
            "kind":candidate["kind"],
            "scope":call_nodes.iter().find(|node| node["id"] == from).map(|node|node["scope"].clone()).unwrap_or(Value::Null),
            "authority":"SOURCE_REFERENCE_CANDIDATE",
            "evidence":evidence
        });
        if let Some(receiver_field) = receiver_field {
            call_edge["receiverField"] = receiver_field;
        }
        call_edges.push(call_edge);
    }
    call_edges.sort_by(|left, right| {
        (
            left["from"].as_str(),
            left["toNode"].as_str(),
            left["targetIdentity"].as_str(),
            left["kind"].as_str(),
            left["authority"].as_str(),
        )
            .cmp(&(
                right["from"].as_str(),
                right["toNode"].as_str(),
                right["targetIdentity"].as_str(),
                right["kind"].as_str(),
                right["authority"].as_str(),
            ))
    });

    let mut constants = Vec::new();
    for reference in packet_record["referencedOwnerFields"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|group| group["fieldReferences"].as_array().into_iter().flatten())
        .filter_map(Value::as_str)
    {
        let (label, row) = selected_by_reference
            .get(reference)
            .ok_or_else(|| invalid("reader packet owner field reference is not selected"))?;
        let normalized = &row["record"]["normalized"];
        let modifiers = array_or_empty(&normalized["modifiers"]);
        if has_modifier(&modifiers, "STATIC") && has_modifier(&modifiers, "FINAL") {
            cite(&mut citations, label, "captured constant declaration");
            constants.push(json!({
                "ownerIdentity":normalized["ownerIdentity"],
                "name":normalized["name"],
                "scope":normalized["scope"],
                "typeDescriptor":retained_declared_type_descriptor(normalized),
                "modifiers":modifiers,
                "annotations":array_or_empty(&normalized["annotations"]),
                "sourceTokens":array_or_empty(&normalized["sourceTokens"]),
                "evidence":[label]
            }));
        }
    }
    constants.sort_by(|left, right| left["name"].as_str().cmp(&right["name"].as_str()));

    let limitations: Vec<Value> = packet_record["gaps"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|gap| json!({"code":gap["code"],"count":gap["count"]}))
        .collect();
    let endpoint_symbol = endpoint_record
        .and_then(|record| record["symbol"].as_str())
        .unwrap_or(entrypoint_id);
    let title = format!("{} · {}", work.subject, endpoint_symbol);
    let mut packet = json!({
        "schema":PACKET_SCHEMA,
        "profile":endpoint_context::PROFILE,
        "authority":"IMMUTABLE_WORK_CAPTURE_NOT_REVERIFIED",
        "audience":&work.request.audience,
        "documentationLanguage":work.request.documentation_language(),
        "notice":"This packet projects saved Work evidence. It is not an accepted narrative, a publication, or execution evidence.",
        "title":title,
        "summary":"Compact source-backed context for authoring one captured HTTP endpoint.",
        "endpoint":{
            "symbol":endpoint_record.map(|record|record["symbol"].clone()),
            "trigger":endpoint_record.map(|record|record["trigger"].clone()),
            "boundaries":endpoint_record.map(|record|record["boundaries"].clone()).unwrap_or(json!([])),
            "evidence":endpoint_evidence
        },
        "types":type_groups,
        "callMap":{
            "authority":packet_record["callGraph"]["authority"],
            "order":"NOT_EXECUTION_ORDER",
            "nodes":call_nodes,
            "edges":call_edges
        },
        "methodSources":method_sources,
        "methodBodies":method_bodies,
        "constants":constants,
        "coverage":{
            "coverage":coverage_record["coverage"],
            "runtimeMode":coverage_record["runtimeMode"],
            "boundaries":coverage_record["boundaries"],
            "callAuthority":coverage_record["callAuthority"],
            "evidence":[coverage_label]
        },
        "limitations":limitations,
        "interpretationLimits":[
            "Annotation names and source tokens are copied from saved declarations; annotation argument semantics are not inferred.",
            "@NotNull indicates declared nullability; it does not establish a nonzero numeric value.",
            "A Java null value does not establish that its JSON property may be omitted.",
            "Method names do not establish runtime behavior or side effects.",
            "A REFERENCES edge identifies a compiler-resolved callback target; it does not establish invocation, timing, or execution order.",
            "A FIELD_RECEIVER_CALL candidate uses a declared field type and a unique captured same-scope method; it does not resolve injection, inheritance, overrides, or runtime dispatch."
        ],
        "runtimeAndSerialization":"UNKNOWN_FROM_THIS_PACKET",
        "citations":citations
    });

    let packet_digest = digest(&packet)?;
    packet["packetDigest"] = json!(packet_digest);
    let selected_rows_digest = digest(&selected_bindings)?;
    let work_id = work.id.clone();
    let snapshot = work.snapshot.clone();
    let profile = work.request.context_profile.clone().unwrap_or_default();
    let binding_digest = digest(&(
        work_id.as_str(),
        snapshot.as_deref(),
        profile.as_str(),
        work.checked.context_digest.as_str(),
        work.checked.input_digest.as_str(),
        selected_rows_digest.as_str(),
        packet_digest.as_str(),
    ))?;
    let mut audit = json!({
        "schema":AUDIT_SCHEMA,
        "purpose":"VERIFICATION_ONLY_NOT_DELIVERED_TO_AUTHOR",
        "workId":work_id,
        "snapshot":snapshot,
        "profile":profile,
        "contextDigest":work.checked.context_digest,
        "inputDigest":work.checked.input_digest,
        "packetDigest":packet_digest,
        "selectedRowsDigest":selected_rows_digest,
        "bindingDigest":binding_digest,
        "records":records
    });
    let audit_digest = digest(&audit)?;
    audit["auditDigest"] = json!(audit_digest);
    Ok((packet, audit))
}

fn synthetic_label(kind: &str, counts: &mut BTreeMap<String, usize>) -> String {
    let (key, prefix) = match kind {
        "ENDPOINT_CONTEXT_PACKET" => ("packet", "p"),
        "COVERAGE" => ("coverage", "c"),
        "REVIEW_REASON" => ("review", "r"),
        "EXTERNAL_INPUT" => ("external", "x"),
        "OBLIGATION" => ("obligation", "o"),
        "SECTION" => ("section", "q"),
        _ => ("other", "u"),
    };
    let next = counts.entry(key.to_owned()).or_default();
    *next += 1;
    format!("{prefix}{next}")
}

fn rows_by_kind<'a>(
    rows: &'a BTreeMap<(String, String), (String, Value)>,
    kind: &str,
) -> Option<(&'a String, &'a Value)> {
    rows.iter()
        .find(|((row_kind, _), _)| row_kind == kind)
        .map(|(_, (label, row))| (label, row))
}

fn array_or_empty(value: &Value) -> Value {
    value
        .as_array()
        .map_or_else(|| json!([]), |values| json!(values))
}

/// Compiler FIELD facts use `jvmDescriptor`; older synthetic/source-syntax
/// records may instead retain `typeDescriptor`. Preserve the captured value
/// verbatim and leave it unknown when neither form is present.
fn retained_declared_type_descriptor(normalized: &Value) -> Value {
    if normalized["jvmDescriptor"]
        .as_str()
        .is_some_and(|descriptor| !descriptor.is_empty())
    {
        return normalized["jvmDescriptor"].clone();
    }
    normalized
        .get("typeDescriptor")
        .filter(|descriptor| !descriptor.is_null())
        .cloned()
        .unwrap_or(Value::Null)
}

fn has_modifier(modifiers: &Value, expected: &str) -> bool {
    modifiers
        .as_array()
        .is_some_and(|items| items.iter().any(|item| item.as_str() == Some(expected)))
}

fn cite(citations: &mut BTreeMap<String, String>, label: &str, role: &str) {
    citations
        .entry(label.to_owned())
        .or_insert_with(|| role.to_owned());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canonical;

    fn add_field_receiver_field(work: &mut Work, id: &str, name: &str, descriptor: &str) {
        let owner = "class:orders.Service";
        let identity = format!("field:{owner}#{name}:{descriptor}");
        let normalized = json!({
            "schema":"codeclew-java-compiler-fact/1.0",
            "declarationKind":"FIELD",
            "symbolIdentity":identity,
            "ownerIdentity":owner,
            "name":name,
            "scope":":main",
            "jvmDescriptor":descriptor,
            "modifiers":[],
            "annotations":[],
            "sourceTokens":["private", "Field", name, ";"]
        });
        let fact_digest = digest(&normalized).unwrap();
        let observation = crate::documentation::model::Observation {
            id: id.into(),
            kind: "SYMBOL".into(),
            service: "orders".into(),
            symbol: identity,
            normalized,
            digest: fact_digest.clone(),
            source_ids: vec!["service-source".into()],
        };
        work.checked
            .services
            .get_mut("orders")
            .unwrap()
            .observations
            .insert(id.into(), observation.clone());
        work.checked.dependencies.insert(id.into(), observation);
        work.influence.insert(id.into(), fact_digest);
        work.handles.insert(
            format!("dependency-{id}"),
            super::super::work::Handle {
                kind: "DEPENDENCY".into(),
                id: id.into(),
            },
        );
    }

    fn add_field_receiver_method(work: &mut Work) {
        let owner = "class:orders.Worker";
        let name = "run";
        let descriptor = "(I)V";
        let identity = format!("method:{owner}#{name}{descriptor}");
        let normalized = json!({
            "schema":"codeclew-java-compiler-fact/1.0",
            "declarationKind":"METHOD",
            "symbolIdentity":identity,
            "ownerIdentity":owner,
            "name":name,
            "scope":":main",
            "jvmDescriptor":descriptor
        });
        let fact_digest = digest(&normalized).unwrap();
        let observation = crate::documentation::model::Observation {
            id: "worker-run-method".into(),
            kind: "SYMBOL".into(),
            service: "orders".into(),
            symbol: identity,
            normalized,
            digest: fact_digest.clone(),
            source_ids: vec!["service-source".into()],
        };
        work.checked
            .services
            .get_mut("orders")
            .unwrap()
            .observations
            .insert(observation.id.clone(), observation.clone());
        work.checked
            .dependencies
            .insert(observation.id.clone(), observation);
        work.influence
            .insert("worker-run-method".into(), fact_digest);
        work.handles.insert(
            "dependency-worker-run-method".into(),
            super::super::work::Handle {
                kind: "DEPENDENCY".into(),
                id: "worker-run-method".into(),
            },
        );
    }

    fn packet_field<'a>(packet: &'a Value, name: &str) -> &'a Value {
        packet["types"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|group| group["fields"].as_array().unwrap())
            .find(|field| field["name"] == name)
            .unwrap()
    }

    fn collect_evidence(value: &Value, out: &mut BTreeSet<String>) {
        match value {
            Value::Object(object) => {
                for (key, child) in object {
                    if key == "evidence"
                        && let Some(items) = child.as_array()
                    {
                        out.extend(items.iter().filter_map(Value::as_str).map(str::to_owned));
                    }
                    collect_evidence(child, out);
                }
            }
            Value::Array(items) => {
                for child in items {
                    collect_evidence(child, out);
                }
            }
            _ => {}
        }
    }

    #[test]
    fn compact_packet_is_deterministic_bound_and_smaller_than_full_profile_rows() {
        let work = super::super::work::api_contract_tests::endpoint_context_fixture();
        let (packet, audit) = build(&work).unwrap();
        let (again, audit_again) = build(&work).unwrap();
        assert_eq!(packet, again);
        assert_eq!(audit, audit_again);

        let rows = endpoint_context::profile_rows(&work).unwrap();
        let baseline_bytes = canonical::bytes(&json!({"items":rows})).unwrap().len();
        let packet_bytes = canonical::bytes(&packet).unwrap().len();
        assert!(
            packet_bytes < baseline_bytes,
            "packet envelope {packet_bytes} bytes should be smaller than full selected rows {baseline_bytes} bytes"
        );
        assert_eq!(packet["profile"], "endpoint-context-v3");
        assert_eq!(packet["callMap"]["order"], "NOT_EXECUTION_ORDER");
        assert_eq!(
            packet["runtimeAndSerialization"],
            "UNKNOWN_FROM_THIS_PACKET"
        );
        assert!(
            packet["notice"]
                .as_str()
                .unwrap()
                .contains("not an accepted narrative")
        );
        assert!(
            packet["citations"]
                .as_object()
                .is_some_and(|c| !c.is_empty())
        );

        let audit_labels: BTreeSet<_> = audit["records"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|row| row["label"].as_str().map(str::to_owned))
            .collect();
        let citations: BTreeSet<_> = packet["citations"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        assert!(citations.is_subset(&audit_labels));
        let mut cited = BTreeSet::new();
        collect_evidence(&packet, &mut cited);
        assert_eq!(citations, cited);
        assert!(
            audit["records"]
                .as_array()
                .unwrap()
                .iter()
                .all(|row| { row["deliveredToAuthor"] == false && row["row"].is_object() })
        );
        assert_eq!(
            audit["purpose"],
            "VERIFICATION_ONLY_NOT_DELIVERED_TO_AUTHOR"
        );
        let mut unsigned_packet = packet.clone();
        unsigned_packet
            .as_object_mut()
            .unwrap()
            .remove("packetDigest");
        assert_eq!(packet["packetDigest"], digest(&unsigned_packet).unwrap());
        assert_eq!(audit["packetDigest"], packet["packetDigest"]);
        assert!(
            audit["bindingDigest"]
                .as_str()
                .unwrap()
                .starts_with("sha256:")
        );
        assert!(
            audit["auditDigest"]
                .as_str()
                .unwrap()
                .starts_with("sha256:")
        );

        let source_labels: Vec<_> = packet["methodSources"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|source| source["reference"].as_str())
            .collect();
        assert_eq!(
            source_labels.iter().copied().collect::<BTreeSet<_>>().len(),
            source_labels.len()
        );
        assert!(source_labels.iter().all(|label| {
            audit["records"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["label"] == *label && row["kind"] == "SOURCE")
        }));
        let packet_text = String::from_utf8(canonical::bytes(&packet).unwrap()).unwrap();
        assert_eq!(packet_text.matches("this.notifyEvent").count(), 1);
        assert!(
            packet["limitations"]
                .as_array()
                .unwrap()
                .iter()
                .any(|gap| gap["code"] == "SOURCE_REFERENCE_AMBIGUOUS_OVERLOAD")
        );
        assert!(
            packet["callMap"]["edges"]
                .as_array()
                .unwrap()
                .iter()
                .any(|edge| { edge["authority"] == "SOURCE_REFERENCE_CANDIDATE" })
        );
        assert!(packet["coverage"]["coverage"].is_string());
    }

    #[test]
    fn field_receiver_candidate_delivers_its_field_evidence_and_one_target_body() {
        let mut work = super::super::work::api_contract_tests::endpoint_context_fixture();
        let evidence = work.checked.services.get_mut("orders").unwrap();
        let source = evidence.sources.get_mut("service-source").unwrap();
        let display_declarations = (0..9)
            .map(|index| format!("  String display{index:02};\n"))
            .collect::<String>();
        let mut source_text = source.text.replacen(
            "class Service {\n",
            &format!("class Service {{\n  Worker worker;\n{display_declarations}"),
            1,
        );
        let display_reads = (0..9)
            .map(|index| format!("      this.display{index:02};\n"))
            .collect::<String>();
        source_text = source_text.replacen(
            "    try {\n",
            &format!("      this.worker.run(1);\n{display_reads}      try {{\n"),
            1,
        );
        source_text.push_str("\nclass Worker { void run(int value) {} }\n");
        source.text = source_text.clone();
        source.text_digest = crate::canonical::hash_bytes(source_text.as_bytes());
        source.end_line = source_text.lines().count().max(1) as u64;
        for index in 0..9 {
            let id = format!("a-display-field-{index:02}");
            let name = format!("display{index:02}");
            add_field_receiver_field(&mut work, &id, &name, "Ljava/lang/String;");
        }
        add_field_receiver_field(
            &mut work,
            "z-receiver-worker-field",
            "worker",
            "Lorders/Worker;",
        );
        add_field_receiver_method(&mut work);

        let rows = endpoint_context::profile_rows(&work).unwrap();
        let profile = rows
            .iter()
            .find(|row| row["kind"] == "ENDPOINT_CONTEXT_PACKET")
            .unwrap();
        let receiver_reference = work
            .handles
            .iter()
            .find(|(_, handle)| handle.id == "z-receiver-worker-field")
            .map(|(reference, _)| reference)
            .unwrap();
        assert!(
            rows.iter().any(|row| {
                row["kind"] == "DEPENDENCY" && row["id"] == "z-receiver-worker-field"
            }),
            "receiver FIELD evidence must be selected outside the owner-field display cap"
        );
        assert!(
            profile["record"]["referencedOwnerFields"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|group| group["fieldReferences"].as_array().unwrap())
                .all(|reference| reference != receiver_reference)
        );

        let (packet, _) = build(&work).unwrap();
        let edge = packet["callMap"]["edges"]
            .as_array()
            .unwrap()
            .iter()
            .find(|edge| edge["kind"] == "FIELD_RECEIVER_CALL")
            .unwrap();
        assert_eq!(edge["authority"], "SOURCE_REFERENCE_CANDIDATE");
        assert_eq!(edge["receiverField"]["name"], "worker");
        assert_eq!(edge["receiverField"]["typeDescriptor"], "Lorders/Worker;");
        let field_label = edge["receiverField"]["evidence"][0].as_str().unwrap();
        assert!(
            edge["evidence"]
                .as_array()
                .unwrap()
                .iter()
                .any(|label| label == field_label)
        );
        assert!(
            packet["citations"][field_label]
                .as_str()
                .unwrap()
                .contains("declared field type")
        );
        let target_node = edge["toNode"].as_str().unwrap();
        assert_eq!(
            packet["callMap"]["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|node| node["id"] == target_node)
                .count(),
            1
        );
        assert_eq!(
            packet["methodBodies"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|body| {
                    body["nodes"]
                        .as_array()
                        .is_some_and(|nodes| nodes.iter().any(|node| node == target_node))
                })
                .count(),
            1
        );
        assert!(
            packet["interpretationLimits"]
                .as_array()
                .unwrap()
                .iter()
                .any(|limit| limit
                    .as_str()
                    .unwrap()
                    .contains("does not resolve injection"))
        );
    }

    #[test]
    fn missing_method_source_stays_a_gap_and_annotation_facts_are_verbatim() {
        let mut work = super::super::work::api_contract_tests::endpoint_context_fixture();
        let endpoint = work.checked.services.get_mut("orders").unwrap();
        let method = endpoint.observations.get_mut("service-method-0").unwrap();
        method.source_ids.clear();
        let field = endpoint.observations.get_mut("request-name-field").unwrap();
        field.normalized["annotations"] = json!([
            "class:jakarta.validation.constraints.NotNull",
            "class:jakarta.validation.constraints.Min"
        ]);
        field.normalized["sourceTokens"] = json!([
            "@", "NotNull", "@", "Min", "(", "0", ")", "private", "Long", "name", ";"
        ]);
        let (packet, _) = build(&work).unwrap();
        assert!(
            packet["limitations"]
                .as_array()
                .unwrap()
                .iter()
                .any(|gap| gap["code"] == "METHOD_BODY_SOURCE_UNAVAILABLE")
        );
        let field = packet["types"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|group| group["fields"].as_array().unwrap())
            .find(|field| field["name"] == "name")
            .unwrap();
        assert_eq!(
            field["annotations"][0],
            "class:jakarta.validation.constraints.NotNull"
        );
        assert_eq!(field["sourceTokens"][5], "0");
        assert!(
            packet["interpretationLimits"]
                .as_array()
                .unwrap()
                .iter()
                .any(|note| note.as_str().unwrap().contains("nonzero numeric value"))
        );
        assert!(
            packet["interpretationLimits"]
                .as_array()
                .unwrap()
                .iter()
                .any(|note| note.as_str().unwrap().contains("may be omitted"))
        );
        assert!(
            packet["interpretationLimits"]
                .as_array()
                .unwrap()
                .iter()
                .any(|note| note.as_str().unwrap().contains("Method names"))
        );
    }

    #[test]
    fn declared_field_descriptors_prefer_compiler_fact_and_preserve_fallback_or_absence() {
        let mut work = super::super::work::api_contract_tests::endpoint_context_fixture();
        {
            let endpoint = work.checked.services.get_mut("orders").unwrap();

            // Match the compiler FIELD shape: the declared type is captured
            // under jvmDescriptor, with no typeDescriptor alias.
            let request_name = endpoint.observations.get_mut("request-name-field").unwrap();
            request_name
                .normalized
                .as_object_mut()
                .unwrap()
                .remove("typeDescriptor");
            request_name.normalized["jvmDescriptor"] = json!("Ljava/lang/String;");

            // Constants use the same compiler FIELD representation.
            let constant = endpoint.observations.get_mut("default-code-field").unwrap();
            constant
                .normalized
                .as_object_mut()
                .unwrap()
                .remove("typeDescriptor");
            constant.normalized["jvmDescriptor"] = json!("Ljava/lang/String;");

            // Retain a legacy source-syntax type on another DTO field.
            let response_status = endpoint
                .observations
                .get_mut("response-status-field")
                .unwrap();
            response_status.normalized["typeDescriptor"] = json!("int");
            response_status
                .normalized
                .as_object_mut()
                .unwrap()
                .remove("jvmDescriptor");
        }

        let (packet, _) = build(&work).unwrap();
        assert_eq!(
            packet_field(&packet, "name")["typeDescriptor"],
            "Ljava/lang/String;"
        );
        assert_eq!(packet_field(&packet, "status")["typeDescriptor"], "int");
        let constant = packet["constants"]
            .as_array()
            .unwrap()
            .iter()
            .find(|constant| constant["name"] == "DEFAULT_CODE")
            .unwrap();
        assert_eq!(constant["typeDescriptor"], "Ljava/lang/String;");
        assert_eq!(
            retained_declared_type_descriptor(&json!({
                "jvmDescriptor":"Ljava/lang/Long;",
                "typeDescriptor":"Long"
            })),
            "Ljava/lang/Long;"
        );

        // Source tokens alone are not a declared type descriptor.
        work.checked
            .services
            .get_mut("orders")
            .unwrap()
            .observations
            .get_mut("response-status-field")
            .unwrap()
            .normalized
            .as_object_mut()
            .unwrap()
            .remove("typeDescriptor");
        let (packet_without_descriptor, _) = build(&work).unwrap();
        assert!(packet_field(&packet_without_descriptor, "status")["typeDescriptor"].is_null());
        assert!(
            retained_declared_type_descriptor(&json!({
                "sourceTokens":["int", "status"]
            }))
            .is_null()
        );
    }
}
