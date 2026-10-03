//! Compact author-facing projection of one immutable endpoint Work profile.
//!
//! This module deliberately does not resolve source, page through Work, or
//! record read receipts. The full selected rows are retained only in the
//! optional audit value returned alongside the compact packet.

use super::{digest, endpoint_context, invalid, model::Source, source_steps, work::Work};
use crate::error::ClewError;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub const PACKET_SCHEMA: &str = "codeclew-documentation-reader-packet/1.0";
pub const AUDIT_SCHEMA: &str = "codeclew-documentation-reader-packet-audit/1.0";

/// Build the short author packet and a separate audit projection of every
/// immutable profile row selected for it.
pub(super) fn build(work: &Work) -> Result<(Value, Value), ClewError> {
    if work.request.context_profile.as_deref() == Some("process-graph-v1") {
        return build_process_graph(work);
    }
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

    let owner_fields = if work.request.authoring_contract.as_deref()
        == Some(super::operation_answer::AUTHORING_CONTRACT)
    {
        let mut field_references = BTreeSet::<String>::new();
        for group in packet_record["referencedOwnerFields"]
            .as_array()
            .into_iter()
            .flatten()
        {
            field_references.extend(
                group["fieldReferences"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_owned),
            );
        }
        let mut fields = Vec::with_capacity(field_references.len());
        for reference in field_references {
            let (label, row) = selected_by_reference
                .get(&reference)
                .ok_or_else(|| invalid("reader packet owner field reference is not selected"))?;
            let record = &row["record"];
            let normalized = &record["normalized"];
            if record["kind"] != "SYMBOL" || normalized["declarationKind"] != "FIELD" {
                return Err(invalid(
                    "reader packet owner field reference is not a FIELD declaration",
                ));
            }
            cite(&mut citations, label, "captured owner field declaration");
            fields.push(json!({
                "reference":reference,
                "ownerIdentity":normalized["ownerIdentity"],
                "name":normalized["name"],
                "scope":normalized["scope"],
                "typeDescriptor":retained_declared_type_descriptor(normalized),
                "modifiers":array_or_empty(&normalized["modifiers"]),
                "annotations":array_or_empty(&normalized["annotations"]),
                "sourceTokens":array_or_empty(&normalized["sourceTokens"]),
                "evidence":[label]
            }));
        }
        fields.sort_by(|left, right| {
            left["ownerIdentity"]
                .as_str()
                .cmp(&right["ownerIdentity"].as_str())
                .then_with(|| left["scope"].as_str().cmp(&right["scope"].as_str()))
                .then_with(|| left["name"].as_str().cmp(&right["name"].as_str()))
                .then_with(|| left["reference"].as_str().cmp(&right["reference"].as_str()))
        });
        Some(fields)
    } else {
        None
    };

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
    let mut interpretation_limits = vec![
        "Annotation names and source tokens are copied from saved declarations; annotation argument semantics are not inferred.",
        "@NotNull indicates declared nullability; it does not establish a nonzero numeric value.",
        "A Java null value does not establish that its JSON property may be omitted.",
        "Method names do not establish runtime behavior or side effects.",
        "A REFERENCES edge identifies a compiler-resolved callback target; it does not establish invocation, timing, or execution order.",
        "A FIELD_RECEIVER_CALL candidate uses a declared field type and a unique captured same-scope method; it does not resolve injection, inheritance, overrides, or runtime dispatch.",
    ];
    if owner_fields
        .as_ref()
        .is_some_and(|fields| !fields.is_empty())
    {
        interpretation_limits.push(
            "Owner field sourceTokens preserve declaration and initializer syntax; they do not establish runtime values or initialization timing, and final does not establish deep immutability.",
        );
    }
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
        "interpretationLimits":interpretation_limits,
        "runtimeAndSerialization":"UNKNOWN_FROM_THIS_PACKET",
        "citations":citations
    });
    if let Some(fields) = owner_fields {
        packet["fields"] = json!(fields);
    }

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

/// Rebuild only the source audit from the immutable Work, preserving the exact
/// packet saved in an author invocation. No current source or latest Check is used.
pub(super) fn audit_saved_packet(work: &Work, packet: &Value) -> Result<Value, ClewError> {
    let profile = work.request.context_profile.as_deref().unwrap_or_default();
    if packet["schema"] != PACKET_SCHEMA || packet["profile"] != profile {
        return Err(invalid(
            "saved reader packet profile differs from immutable Work",
        ));
    }
    let packet_digest = packet["packetDigest"]
        .as_str()
        .ok_or_else(|| invalid("saved reader packet has no digest"))?;
    let mut unsigned = packet.clone();
    unsigned
        .as_object_mut()
        .ok_or_else(|| invalid("saved packet is not an object"))?
        .remove("packetDigest");
    if digest(&unsigned)? != packet_digest {
        return Err(invalid("saved reader packet content digest differs"));
    }
    let rows = match profile {
        endpoint_context::PROFILE => endpoint_context::profile_rows(work)?,
        "process-graph-v1" => endpoint_context::process_profile_rows(work)?,
        _ => return Err(invalid("unsupported saved reader packet profile")),
    };
    let mut counts = BTreeMap::new();
    let mut labels = BTreeSet::new();
    let mut records = Vec::new();
    let mut selected = Vec::new();
    for row in rows {
        let kind = row["kind"]
            .as_str()
            .ok_or_else(|| invalid("audit row has no kind"))?;
        let id = row["id"]
            .as_str()
            .ok_or_else(|| invalid("audit row has no identity"))?;
        let reference = work
            .handles
            .iter()
            .find(|(_, handle)| handle.kind == kind && handle.id == id)
            .map(|(reference, _)| reference.clone());
        let label = reference
            .clone()
            .unwrap_or_else(|| synthetic_label(kind, &mut counts));
        if !labels.insert(label.clone()) {
            return Err(invalid("saved audit labels are ambiguous"));
        }
        let record_digest = digest(&row)?;
        selected.push(json!({"label":label,"recordDigest":record_digest}));
        records.push(
            json!({"label":label,"kind":kind,"id":id,"workReference":reference,
            "recordDigest":record_digest,"deliveredToAuthor":false,"row":row}),
        );
    }
    let citations = packet["citations"]
        .as_object()
        .ok_or_else(|| invalid("saved packet has no citations"))?;
    if citations.keys().any(|label| !labels.contains(label)) {
        return Err(invalid(
            "saved packet citation is absent from immutable Work",
        ));
    }
    let selected_rows_digest = digest(&selected)?;
    let binding_digest = digest(&(
        work.id.as_str(),
        work.snapshot.as_deref(),
        profile,
        work.checked.context_digest.as_str(),
        work.checked.input_digest.as_str(),
        selected_rows_digest.as_str(),
        packet_digest,
    ))?;
    let mut audit = json!({"schema":AUDIT_SCHEMA,"purpose":"VERIFICATION_ONLY_NOT_DELIVERED_TO_AUTHOR",
        "workId":work.id,"snapshot":work.snapshot,"profile":profile,
        "contextDigest":work.checked.context_digest,"inputDigest":work.checked.input_digest,
        "packetDigest":packet_digest,"selectedRowsDigest":selected_rows_digest,
        "bindingDigest":binding_digest,"records":records});
    if profile == "process-graph-v1" {
        audit["processGraph"] = super::process_graph::collect_from_work(work)?;
    }
    audit["auditDigest"] = json!(digest(&audit)?);
    Ok(audit)
}

fn build_process_graph(work: &Work) -> Result<(Value, Value), ClewError> {
    let rows = endpoint_context::process_profile_rows(work)?;
    let graph = super::process_graph::collect_from_work(work)?;
    let mut row_counts = BTreeMap::<String, usize>::new();
    let mut selected_by_key = BTreeMap::<(String, String), (String, Value)>::new();
    let mut selected_by_reference = BTreeMap::<String, (String, Value)>::new();
    let mut records = Vec::with_capacity(rows.len());
    let mut selected_bindings = Vec::with_capacity(rows.len());
    let mut used_labels = BTreeSet::new();

    for row in rows {
        let kind = row["kind"]
            .as_str()
            .ok_or_else(|| invalid("process packet selected row has no kind"))?
            .to_owned();
        let id = row["id"]
            .as_str()
            .ok_or_else(|| invalid("process packet selected row has no id"))?
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
            return Err(invalid("process packet evidence labels are ambiguous"));
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
        selected_by_key.insert((kind, id), (label.clone(), row.clone()));
        if let Some(reference) = work_reference {
            selected_by_reference.insert(reference, (label, row));
        }
    }

    let process_row = rows_by_kind(&selected_by_key, "PROCESS_CONTEXT_PACKET")
        .ok_or_else(|| invalid("process-graph-v1 selected no process context row"))?;
    let process_label = process_row.0.clone();
    let process_record = &process_row.1["record"];
    let coverage_row = rows_by_kind(&selected_by_key, "COVERAGE")
        .ok_or_else(|| invalid("process-graph-v1 selected no coverage row"))?;
    let coverage_label = coverage_row.0.clone();
    let coverage_record = &coverage_row.1["record"];
    let mut citations = BTreeMap::<String, String>::new();
    cite(
        &mut citations,
        &process_label,
        "selected internal process context",
    );
    cite(&mut citations, &coverage_label, "saved capture coverage");

    let root = &process_record["root"];
    let root_reference = process_record["rootDeclarationReference"]
        .as_str()
        .ok_or_else(|| invalid("process context has no root declaration reference"))?;
    let (root_label, _) = selected_by_reference
        .get(root_reference)
        .ok_or_else(|| invalid("process root declaration is not selected"))?;
    cite(&mut citations, root_label, "exact internal process root");

    let mut method_ids = BTreeMap::<(String, String), String>::new();
    let mut method_identities = BTreeMap::<String, String>::new();
    let mut methods = Vec::new();
    let mut method_sources = Vec::new();
    let mut emitted_sources = BTreeSet::<String>::new();
    for node in process_record["callGraph"]["nodes"]
        .as_array()
        .into_iter()
        .flatten()
    {
        let identity = node["symbolIdentity"].as_str().unwrap_or_default();
        let scope = node["scope"].as_str().unwrap_or_default();
        let node_id = node["id"].as_str().unwrap_or_default();
        method_ids.insert((identity.to_owned(), scope.to_owned()), node_id.to_owned());
        method_identities.insert(node_id.to_owned(), identity.to_owned());
        let declaration_reference = node["declarationReference"]
            .as_str()
            .ok_or_else(|| invalid("selected callable has no declaration reference"))?;
        let (declaration_label, _) = selected_by_reference
            .get(declaration_reference)
            .ok_or_else(|| invalid("selected callable declaration reference is unavailable"))?;
        cite(
            &mut citations,
            declaration_label,
            "selected callable declaration",
        );

        let source_reference = node["bodyReference"].as_str();
        let mut body = Value::Null;
        let mut evidence = vec![declaration_label.clone()];
        if let Some(source_reference) = source_reference
            && let Some((source_label, source_row)) = selected_by_reference.get(source_reference)
        {
            let source = &source_row["record"];
            cite(&mut citations, source_label, "retained callable source");
            evidence.push(source_label.clone());
            if emitted_sources.insert(source_reference.to_owned()) {
                method_sources.push(json!({
                    "reference":source_reference,
                    "authority":source["authority"],
                    "text":source["text"],
                    "evidence":[source_label]
                }));
            }
            if let Some(text) = source["text"].as_str()
                && let Some((start, end)) = source_steps::method_body(text, identity)
            {
                body = json!({
                    "sourceReference":source_reference,
                    "startByte":start,
                    "endByte":end,
                    "evidence":[source_label]
                });
            }
        }
        methods.push(json!({
            "id":node_id,
            "symbolIdentity":identity,
            "ownerIdentity":node["ownerIdentity"],
            "name":node["name"],
            "scope":scope,
            "declarationReference":declaration_reference,
            "body":body,
            "evidence":evidence
        }));
    }

    // Class-level retained source is useful context when a method declaration
    // links to the containing class source but has no method-specific source
    // reference. Share the source once with its type and source evidence; do
    // not re-embed it in every method or type entry.
    for (type_reference, (type_label, type_row)) in &selected_by_reference {
        let record = &type_row["record"];
        if type_row["kind"] != "DEPENDENCY"
            || !matches!(
                record["normalized"]["declarationKind"].as_str(),
                Some("CLASS" | "INTERFACE" | "ENUM" | "RECORD" | "ANNOTATION_TYPE")
            )
        {
            continue;
        }
        for source_id in record["sourceIds"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            let Some(source_reference) = work_reference(work, "SOURCE", source_id) else {
                continue;
            };
            let Some((source_label, source_row)) = selected_by_reference.get(source_reference)
            else {
                continue;
            };
            let source = &source_row["record"];
            cite(
                &mut citations,
                type_label,
                "retained containing type declaration",
            );
            cite(
                &mut citations,
                source_label,
                "retained containing type source",
            );
            if let Some(existing) = method_sources
                .iter_mut()
                .find(|item| item["reference"] == source_reference)
            {
                if existing["contextFor"].is_null() {
                    existing["contextFor"] = json!(type_reference);
                }
                if !existing["contextReferences"].is_array() {
                    existing["contextReferences"] = json!([]);
                }
                let context_references = existing["contextReferences"].as_array_mut().unwrap();
                if !context_references
                    .iter()
                    .any(|reference| reference.as_str() == Some(type_reference))
                {
                    context_references.push(json!(type_reference));
                }
                let source_evidence = existing["evidence"].as_array_mut().unwrap();
                for label in [type_label.as_str(), source_label.as_str()] {
                    if !source_evidence
                        .iter()
                        .any(|evidence| evidence.as_str() == Some(label))
                    {
                        source_evidence.push(json!(label));
                    }
                }
            } else if emitted_sources.insert(source_reference.to_owned()) {
                method_sources.push(json!({
                    "reference":source_reference,
                    "authority":source["authority"],
                    "text":source["text"],
                    "evidence":[type_label,source_label],
                    "contextFor":type_reference,
                    "contextReferences":[type_reference]
                }));
            }
        }
    }

    let mut source_contexts = Vec::new();
    for context in process_record["sourceContexts"]
        .as_array()
        .into_iter()
        .flatten()
    {
        let declaration_reference = context["declarationReference"]
            .as_str()
            .ok_or_else(|| invalid("process source context has no declaration reference"))?;
        let (declaration_label, _) = selected_by_reference
            .get(declaration_reference)
            .ok_or_else(|| invalid("process source context declaration is not selected"))?;
        cite(
            &mut citations,
            declaration_label,
            "source-context candidate declaration",
        );
        let source_reference = context["sourceReference"].as_str();
        let mut evidence = vec![declaration_label.clone()];
        if let Some(source_reference) = source_reference {
            let (source_label, source_row) = selected_by_reference
                .get(source_reference)
                .ok_or_else(|| invalid("process source-context source is not selected"))?;
            let source = &source_row["record"];
            cite(
                &mut citations,
                source_label,
                "retained source-context candidate body",
            );
            evidence.push(source_label.clone());
            if emitted_sources.insert(source_reference.to_owned()) {
                method_sources.push(json!({
                    "reference":source_reference,
                    "authority":source["authority"],
                    "text":source["text"],
                    "evidence":[source_label]
                }));
            }
        }
        let referenced_from_source_reference = context["referencedFromSourceReference"].as_str();
        if let Some(source_reference) = referenced_from_source_reference {
            let (source_label, _) = selected_by_reference
                .get(source_reference)
                .ok_or_else(|| invalid("process source-context origin is not selected"))?;
            cite(
                &mut citations,
                source_label,
                "source that names a context candidate",
            );
            if !evidence.iter().any(|item| item == source_label) {
                evidence.push(source_label.clone());
            }
        }
        source_contexts.push(json!({
            "kind":context["kind"],
            "authority":context["authority"],
            "symbolIdentity":context["symbolIdentity"],
            "ownerIdentity":context["ownerIdentity"],
            "scope":context["scope"],
            "declarationReference":declaration_reference,
            "sourceReference":source_reference,
            "referencedFromSourceReference":referenced_from_source_reference,
            "evidence":evidence
        }));
    }

    let mut edges = Vec::new();
    for fact_reference in process_record["callGraph"]["providerEdgeFactReferences"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        let (label, row) = selected_by_reference
            .get(fact_reference)
            .ok_or_else(|| invalid("process callsite evidence reference is unavailable"))?;
        let record = &row["record"];
        let normalized = &record["normalized"];
        let (source_identity, target_identity, kind, scope, authority) =
            if record["kind"] == "CALL_RELATION" {
                (
                    normalized["sourceIdentity"].as_str().unwrap_or_default(),
                    normalized["targetIdentity"].as_str().unwrap_or_default(),
                    normalized["relationKind"].as_str().unwrap_or("CALLS"),
                    normalized["scope"].as_str().unwrap_or_default(),
                    if normalized["resolution"] == "COMPILER_EXACT" {
                        "COMPILER_EXACT_PROVIDER_RELATION"
                    } else {
                        "UNVERIFIED_PROVIDER_RELATION"
                    },
                )
            } else {
                (
                    record["symbol"].as_str().unwrap_or_default(),
                    normalized["target"].as_str().unwrap_or_default(),
                    normalized["kind"].as_str().unwrap_or("CALL"),
                    normalized["scope"].as_str().unwrap_or_default(),
                    "RETAINED_FLOW_TARGET",
                )
            };
        let from = method_ids
            .get(&(source_identity.to_owned(), scope.to_owned()))
            .cloned();
        let to = method_ids
            .get(&(target_identity.to_owned(), scope.to_owned()))
            .cloned();
        cite(&mut citations, label, "retained callsite evidence");
        let source_reference = record["sourceIds"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .find_map(|id| {
                let reference = work_reference(work, "SOURCE", id)?;
                selected_by_reference
                    .contains_key(reference)
                    .then(|| reference.to_owned())
            });
        let mut edge_evidence = vec![label.clone()];
        if let Some(source_reference) = source_reference.as_deref()
            && let Some((source_label, _)) = selected_by_reference.get(source_reference)
        {
            cite(&mut citations, source_label, "retained callsite source");
            edge_evidence.push(source_label.clone());
        }
        edges.push(json!({
            "id":format!("fact:{fact_reference}"),
            "fromMethodId":from,
            "targetMethodId":to,
            "targetIdentity":target_identity,
            "kind":kind,
            "scope":scope,
            "authority":authority,
            "callsiteReference":fact_reference,
            "sourceReference":source_reference,
            "evidence":edge_evidence
        }));
    }
    for candidate in process_record["callGraph"]["sourceReferenceCandidates"]
        .as_array()
        .into_iter()
        .flatten()
    {
        let from = candidate["fromNode"].as_str().unwrap_or_default();
        let to = candidate["toNode"].as_str();
        let source_reference = candidate["sourceReference"].as_str();
        let receiver_field_reference = candidate["receiverFieldReference"].as_str();
        let target_identity = to
            .and_then(|target| method_identities.get(target))
            .map_or_else(
                || candidate["targetIdentity"].clone(),
                |identity| json!(identity),
            );
        let mut evidence = vec![process_label.clone()];
        for reference in [source_reference, receiver_field_reference]
            .into_iter()
            .flatten()
        {
            if let Some((label, _)) = selected_by_reference.get(reference) {
                cite(&mut citations, label, "source-context candidate evidence");
                evidence.push(label.clone());
            }
        }
        edges.push(json!({
            "fromMethodId":from,
            "targetMethodId":to,
            "targetIdentity":target_identity,
            "kind":candidate["kind"],
            "authority":"SOURCE_REFERENCE_CANDIDATE",
            "sourceReference":source_reference,
            "receiverFieldReference":receiver_field_reference,
            "evidence":evidence
        }));
    }
    edges.sort_by(|left, right| {
        (
            left["fromMethodId"].as_str(),
            left["targetMethodId"].as_str(),
            left["callsiteReference"].as_str(),
            left["authority"].as_str(),
        )
            .cmp(&(
                right["fromMethodId"].as_str(),
                right["targetMethodId"].as_str(),
                right["callsiteReference"].as_str(),
                right["authority"].as_str(),
            ))
    });

    coalesce_process_method_sources(
        &mut method_sources,
        &mut methods,
        &mut edges,
        &selected_by_reference,
    )?;

    let mut field_refs = BTreeSet::<String>::new();
    for group in process_record["referencedOwnerFields"]
        .as_array()
        .into_iter()
        .flatten()
    {
        field_refs.extend(
            group["fieldReferences"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_owned),
        );
    }
    for candidate in process_record["callGraph"]["sourceReferenceCandidates"]
        .as_array()
        .into_iter()
        .flatten()
    {
        if let Some(reference) = candidate["receiverFieldReference"].as_str() {
            field_refs.insert(reference.to_owned());
        }
    }
    let mut fields = Vec::new();
    for reference in field_refs {
        let (label, row) = selected_by_reference
            .get(&reference)
            .ok_or_else(|| invalid("selected field declaration reference is unavailable"))?;
        let normalized = &row["record"]["normalized"];
        cite(&mut citations, label, "retained field declaration");
        fields.push(json!({
            "reference":reference,
            "ownerIdentity":normalized["ownerIdentity"],
            "name":normalized["name"],
            "typeDescriptor":retained_declared_type_descriptor(normalized),
            "modifiers":array_or_empty(&normalized["modifiers"]),
            "annotations":array_or_empty(&normalized["annotations"]),
            "sourceTokens":array_or_empty(&normalized["sourceTokens"]),
            "evidence":[label]
        }));
    }
    fields.sort_by(|left, right| {
        left["ownerIdentity"]
            .as_str()
            .cmp(&right["ownerIdentity"].as_str())
            .then_with(|| left["name"].as_str().cmp(&right["name"].as_str()))
    });

    let mut types = Vec::new();
    for (reference, (label, row)) in &selected_by_reference {
        if row["record"]["kind"] != "SYMBOL"
            || !matches!(
                row["record"]["normalized"]["declarationKind"].as_str(),
                Some("CLASS" | "INTERFACE" | "ENUM" | "RECORD" | "ANNOTATION_TYPE")
            )
        {
            continue;
        }
        let normalized = &row["record"]["normalized"];
        cite(&mut citations, label, "retained type and class context");
        types.push(json!({
            "reference":reference,
            "symbolIdentity":normalized["symbolIdentity"],
            "declarationKind":normalized["declarationKind"],
            "ownerIdentity":normalized["ownerIdentity"],
            "name":normalized["name"],
            "scope":normalized["scope"],
            "superclass":normalized["superclass"],
            "interfaces":array_or_empty(&normalized["interfaces"]),
            "evidence":[label]
        }));
    }
    types.sort_by(|left, right| {
        left["symbolIdentity"]
            .as_str()
            .cmp(&right["symbolIdentity"].as_str())
    });

    let limitations: Vec<_> = process_record["gaps"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|gap| json!({"code":gap["code"],"count":gap["count"],"examples":gap["examples"]}))
        .collect();
    let root_node = root["declarationId"].as_str().and_then(|_| {
        method_ids
            .get(&(
                root["symbolIdentity"].as_str()?.to_owned(),
                root["scope"].as_str()?.to_owned(),
            ))
            .cloned()
    });
    let process_intent = if work.subject.starts_with("scenario:") {
        let scenario_row = selected_by_key
            .get(&("DEPENDENCY".to_owned(), work.subject.clone()))
            .ok_or_else(|| {
                invalid("frozen process intention is not selected for the author packet")
            })?;
        let scenario_record = &scenario_row.1["record"];
        if scenario_record["kind"] != "SCENARIO_SELECTION" || scenario_record["id"] != work.subject
        {
            return Err(invalid(
                "selected process intention does not match this scenario Work",
            ));
        }
        let definition_reference = scenario_row.0.as_str();
        cite(
            &mut citations,
            definition_reference,
            "frozen saved process intention, not source evidence",
        );
        let definition = &scenario_record["normalized"];
        let process = definition
            .get("process")
            .filter(|process| process.is_object())
            .ok_or_else(|| invalid("frozen scenario has no process intention"))?;
        let mut declared_continuations = Vec::new();
        for interaction_id in definition["interactions"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            let id = format!("interaction:{interaction_id}");
            let (label, row) = selected_by_key
                .get(&("DEPENDENCY".to_owned(), id.clone()))
                .ok_or_else(|| {
                    invalid(format!(
                        "declared process continuation {id} is missing from the selected packet rows"
                    ))
                })?;
            if row["record"]["kind"] != "DECLARED_INTERACTION" {
                return Err(invalid(format!(
                    "selected process continuation {id} is not a declared interaction"
                )));
            }
            cite(
                &mut citations,
                label,
                "declared interaction, not evidence of execution",
            );
            declared_continuations.push(json!({
                "id":interaction_id,
                "reference":label,
                "digest":row["record"]["digest"],
                "authority":"DECLARED_INTERACTION_NOT_EXECUTED",
                "definition":row["record"]["normalized"]
            }));
        }
        let linked_subviews: Vec<Value> = process["linkedSubviews"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(|id| {
                json!({
                    "id":id,
                    "authority":"USER_INTENTION_ONLY_NOT_RESOLVED_AS_A_CONTINUATION"
                })
            })
            .collect();
        Some(json!({
            "authority":"USER_INTENTION_NOT_SOURCE_EVIDENCE",
            "definitionReference":definition_reference,
            "definitionDigest":scenario_record["digest"],
            "title":definition["title"],
            "summary":definition["summary"],
            "scope":process["scope"],
            "trigger":process["trigger"],
            "desiredOutcomes":process["outcomes"],
            "declaredContinuations":declared_continuations,
            "linkedSubviews":linked_subviews
        }))
    } else {
        None
    };
    let mut packet = json!({
        "schema":PACKET_SCHEMA,
        "profile":"process-graph-v1",
        "authority":"IMMUTABLE_WORK_CAPTURE_NOT_REVERIFIED",
        "audience":work.request.audience,
        "documentationLanguage":work.request.documentation_language(),
        "notice":"This packet uses retained source and provider evidence to support an internal process draft. It is not a runtime trace, accepted narrative, review, or publication.",
        "title":format!("{} · {}",work.subject,root["symbolIdentity"].as_str().unwrap_or_default()),
        "question":work.request.question,
        "context":{
            "authority":process_record["authority"],
            "evidence":[process_label]
        },
        "root":{
            "methodId":root_node,
            "symbolIdentity":root["symbolIdentity"],
            "ownerIdentity":root["ownerIdentity"],
            "name":root["name"],
            "scope":root["scope"],
            "declarationReference":root_reference,
            "evidence":[root_label]
        },
        "methods":methods,
        "edges":edges,
        "sourceContexts":source_contexts,
        "fields":fields,
        "types":types,
        "methodSources":method_sources,
        "coverage":{
            "coverage":coverage_record["coverage"],
            "runtimeMode":coverage_record["runtimeMode"],
            "boundaries":coverage_record["boundaries"],
            "callAuthority":coverage_record["callAuthority"],
            "evidence":[coverage_label]
        },
        "limitations":limitations,
        "interpretationLimits":[
            "Retained FLOW and compiler relations preserve provider evidence, not runtime execution or statement timing.",
            "SOURCE_REFERENCE_CANDIDATE edges identify same-scope source-context candidates only; they do not establish executed calls, receiver identity, inheritance dispatch, or order.",
            "Only the supplied root, selected declarations, retained method source, fields, types, and listed frontiers support claims; missing context remains unknown.",
            "A field type or superclass declaration does not establish the runtime object or selected override."
        ],
        "graphAuditBinding":{
            "schema":graph["schema"],
            "artifactDigest":graph["artifactDigest"],
            "rootMethodId":graph["rootMethodId"],
            "snapshot":graph["snapshot"]["handle"],
            "service":graph["snapshot"]["service"]
        },
        "citations":citations
    });
    if let Some(process_intent) = process_intent {
        packet["processIntent"] = process_intent;
    }
    let packet_digest = digest(&packet)?;
    packet["packetDigest"] = json!(packet_digest);
    let selected_rows_digest = digest(&selected_bindings)?;
    let profile = work.request.context_profile.as_deref().unwrap_or_default();
    let binding_digest = digest(&(
        work.id.as_str(),
        work.snapshot.as_deref(),
        profile,
        work.checked.context_digest.as_str(),
        work.checked.input_digest.as_str(),
        selected_rows_digest.as_str(),
        packet_digest.as_str(),
    ))?;
    let mut audit = json!({
        "schema":AUDIT_SCHEMA,
        "purpose":"VERIFICATION_ONLY_NOT_DELIVERED_TO_AUTHOR",
        "workId":work.id,
        "snapshot":work.snapshot,
        "profile":profile,
        "contextDigest":work.checked.context_digest,
        "inputDigest":work.checked.input_digest,
        "packetDigest":packet_digest,
        "selectedRowsDigest":selected_rows_digest,
        "bindingDigest":binding_digest,
        "processGraph":graph,
        "records":records
    });
    let audit_digest = digest(&audit)?;
    audit["auditDigest"] = json!(audit_digest);
    Ok((packet, audit))
}

fn coalesce_process_method_sources(
    method_sources: &mut Vec<Value>,
    methods: &mut [Value],
    edges: &mut [Value],
    selected_by_reference: &BTreeMap<String, (String, Value)>,
) -> Result<(), ClewError> {
    let mut aliases = BTreeMap::<String, (String, usize, usize)>::new();
    let mut ambiguous = BTreeSet::<String>::new();

    for method in methods.iter() {
        let Some(source_reference) = method["body"]["sourceReference"].as_str() else {
            continue;
        };
        let Some(source_entry) = method_sources
            .iter()
            .find(|entry| entry["reference"].as_str() == Some(source_reference))
        else {
            continue;
        };
        if source_entry["contextFor"].is_string() {
            continue;
        }
        let Some((_, source_row)) = selected_by_reference.get(source_reference) else {
            continue;
        };
        let source: Source = serde_json::from_value(source_row["record"].clone())
            .map_err(|_| invalid("process method source record is invalid"))?;
        let owner = method["ownerIdentity"].as_str().unwrap_or_default();
        let scope = method["scope"].as_str().unwrap_or_default();
        if owner.is_empty() || scope.is_empty() {
            continue;
        }

        let mut candidates = Vec::<(String, usize, usize)>::new();
        for context in method_sources.iter().filter(|entry| {
            entry["reference"].as_str() != Some(source_reference) && entry["contextFor"].is_string()
        }) {
            let container_reference = context["reference"].as_str().unwrap_or_default();
            let context_references: Vec<_> = context["contextReferences"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .chain(context["contextFor"].as_str())
                .collect();
            let belongs_to_owner = context_references.iter().any(|type_reference| {
                selected_by_reference
                    .get(*type_reference)
                    .is_some_and(|(_, type_row)| {
                        type_row["record"]["normalized"]["symbolIdentity"] == owner
                            && type_row["record"]["normalized"]["scope"] == scope
                    })
            });
            if !belongs_to_owner {
                continue;
            }
            let Some((_, container_row)) = selected_by_reference.get(container_reference) else {
                continue;
            };
            let container: Source = serde_json::from_value(container_row["record"].clone())
                .map_err(|_| invalid("process containing type source record is invalid"))?;
            if let Some((start, end)) = exact_source_range(&container, &source) {
                candidates.push((container_reference.to_owned(), start, end));
            }
        }

        candidates.sort();
        candidates.dedup();
        match candidates.as_slice() {
            [(container_reference, start, end)] => {
                if aliases.get(source_reference).is_some_and(|existing| {
                    existing != &(container_reference.clone(), *start, *end)
                }) {
                    ambiguous.insert(source_reference.to_owned());
                } else {
                    aliases.insert(
                        source_reference.to_owned(),
                        (container_reference.clone(), *start, *end),
                    );
                }
            }
            [] => {}
            _ => {
                ambiguous.insert(source_reference.to_owned());
            }
        }
    }
    for reference in ambiguous {
        aliases.remove(&reference);
    }
    if aliases.is_empty() {
        return Ok(());
    }

    for (source_reference, (container_reference, start, end)) in &aliases {
        let source_entry = method_sources
            .iter()
            .find(|entry| entry["reference"].as_str() == Some(source_reference))
            .ok_or_else(|| invalid("process source alias target is unavailable"))?;
        let (authority, evidence) = (
            source_entry["authority"].clone(),
            source_entry["evidence"].clone(),
        );
        let container_entry = method_sources
            .iter_mut()
            .find(|entry| entry["reference"].as_str() == Some(container_reference))
            .ok_or_else(|| invalid("process source alias container is unavailable"))?;
        if !container_entry["sourceAliases"].is_array() {
            container_entry["sourceAliases"] = json!([]);
        }
        container_entry["sourceAliases"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "reference":source_reference,
                "authority":authority,
                "startByte":start,
                "endByte":end,
                "evidence":evidence
            }));
    }

    for method in methods {
        let body = &mut method["body"];
        let Some(source_reference) = body["sourceReference"].as_str().map(str::to_owned) else {
            continue;
        };
        let Some((container_reference, start, end)) = aliases.get(&source_reference) else {
            continue;
        };
        let body_start = body["startByte"]
            .as_u64()
            .and_then(|offset| usize::try_from(offset).ok())
            .ok_or_else(|| invalid("process method body start offset is invalid"))?;
        let body_end = body["endByte"]
            .as_u64()
            .and_then(|offset| usize::try_from(offset).ok())
            .ok_or_else(|| invalid("process method body end offset is invalid"))?;
        let source_start = *start;
        let source_end = *end;
        if body_start > body_end || body_end > source_end.saturating_sub(source_start) {
            return Err(invalid(
                "process method body range exceeds its aliased source",
            ));
        }
        body["sourceReference"] = json!(container_reference);
        body["startByte"] = json!(source_start + body_start);
        body["endByte"] = json!(source_start + body_end);
    }
    for edge in edges {
        let Some(source_reference) = edge["sourceReference"].as_str().map(str::to_owned) else {
            continue;
        };
        if let Some((container_reference, _, _)) = aliases.get(&source_reference) {
            edge["sourceReference"] = json!(container_reference);
        }
    }
    method_sources.retain(|entry| {
        entry["reference"]
            .as_str()
            .is_none_or(|reference| !aliases.contains_key(reference))
    });
    Ok(())
}

fn exact_source_range(container: &Source, part: &Source) -> Option<(usize, usize)> {
    if part.text.is_empty()
        || container.service != part.service
        || container.revision != part.revision
        || container.file != part.file
        || container.authority != part.authority
        || container.start_line > part.start_line
        || container.end_line < part.end_line
    {
        return None;
    }
    match (&container.occurrence, &part.occurrence) {
        (Some(container_occurrence), Some(part_occurrence)) => {
            if container_occurrence.snapshot != part_occurrence.snapshot
                || container_occurrence.blob != part_occurrence.blob
            {
                return None;
            }
            super::process_context::covered_text(container, part)
        }
        (None, None) => {
            let mut matches = container.text.char_indices().filter_map(|(start, _)| {
                let end = start.checked_add(part.text.len())?;
                (container.text.get(start..end)? == part.text)
                    .then_some((start, end))
                    .filter(|(start, end)| {
                        source_line_at(container, *start) == Some(part.start_line)
                            && end
                                .checked_sub(1)
                                .and_then(|offset| source_line_at(container, offset))
                                == Some(part.end_line)
                    })
            });
            let first = matches.next()?;
            matches.next().is_none().then_some(first)
        }
        _ => None,
    }
}

fn source_line_at(source: &Source, byte_offset: usize) -> Option<u64> {
    let prefix = source.text.get(..byte_offset)?;
    source
        .start_line
        .checked_add(prefix.bytes().filter(|byte| *byte == b'\n').count() as u64)
}

fn synthetic_label(kind: &str, counts: &mut BTreeMap<String, usize>) -> String {
    let (key, prefix) = match kind {
        "ENDPOINT_CONTEXT_PACKET" => ("packet", "p"),
        "PROCESS_CONTEXT_PACKET" => ("packet", "p"),
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

fn work_reference<'a>(work: &'a Work, kind: &str, id: &str) -> Option<&'a str> {
    work.handles
        .iter()
        .find(|(_, handle)| handle.kind == kind && handle.id == id)
        .map(|(reference, _)| reference.as_str())
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

    fn add_owner_field(
        work: &mut Work,
        id: &str,
        owner: &str,
        name: &str,
        descriptor: &str,
        modifiers: &[&str],
        source_tokens: &[&str],
    ) {
        let identity = format!("field:{owner}#{name}:{descriptor}");
        let normalized = json!({
            "schema":"codeclew-java-compiler-fact/1.0",
            "declarationKind":"FIELD",
            "symbolIdentity":identity,
            "ownerIdentity":owner,
            "name":name,
            "scope":":main",
            "jvmDescriptor":descriptor,
            "modifiers":modifiers,
            "annotations":[],
            "sourceTokens":source_tokens
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

    fn add_field_receiver_field(work: &mut Work, id: &str, name: &str, descriptor: &str) {
        let owner = "class:orders.Service";
        add_owner_field(
            work,
            id,
            owner,
            name,
            descriptor,
            &[],
            &["private", "Field", name, ";"],
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

    fn add_process_profile_fields(work: &mut Work, root_source: &str, shadow: Option<&str>) {
        let service = work.checked.services.get_mut("orders").unwrap();
        let source = service.sources.get_mut("endpoint-source").unwrap();
        source.text = root_source.into();
        source.text_digest = crate::canonical::hash_bytes(source.text.as_bytes());
        source.end_line = source.text.lines().count().max(1) as u64;

        let helper_source_text =
            "class OtherHelper { static boolean predicate() { return true; } }";
        let helper_source = crate::documentation::model::Source {
            id: "other-helper-source".into(),
            service: "orders".into(),
            revision: service.revision.clone(),
            file: "src/OtherHelper.java".into(),
            start_line: 1,
            end_line: 1,
            text: helper_source_text.into(),
            text_digest: crate::canonical::hash_bytes(helper_source_text.as_bytes()),
            evidence_digest: "test-helper-source-evidence".into(),
            authority: "TRANSFORMED_SOURCE".into(),
            occurrence: None,
            url: None,
        };
        service
            .sources
            .insert(helper_source.id.clone(), helper_source);
        work.handles.insert(
            "source-other-helper".into(),
            super::super::work::Handle {
                kind: "SOURCE".into(),
                id: "other-helper-source".into(),
            },
        );
        let sibling_source_text =
            "class OtherHelperSibling { static boolean predicate() { return false; } }";
        let sibling_source = crate::documentation::model::Source {
            id: "sibling-helper-source".into(),
            service: "orders".into(),
            revision: service.revision.clone(),
            file: "src/OtherHelperSibling.java".into(),
            start_line: 1,
            end_line: 1,
            text: sibling_source_text.into(),
            text_digest: crate::canonical::hash_bytes(sibling_source_text.as_bytes()),
            evidence_digest: "test-sibling-source-evidence".into(),
            authority: "TRANSFORMED_SOURCE".into(),
            occurrence: None,
            url: None,
        };
        service
            .sources
            .insert(sibling_source.id.clone(), sibling_source);
        work.handles.insert(
            "source-sibling-helper".into(),
            super::super::work::Handle {
                kind: "SOURCE".into(),
                id: "sibling-helper-source".into(),
            },
        );

        let facts = [
            (
                "other-helper-type",
                "class:orders.OtherHelper",
                json!({
                    "schema":"codeclew-java-compiler-fact/1.0",
                    "declarationKind":"CLASS",
                    "symbolIdentity":"class:orders.OtherHelper",
                    "ownerIdentity":"class:orders",
                    "name":"OtherHelper",
                    "scope":":main"
                }),
                vec!["other-helper-source".to_owned()],
            ),
            (
                "other-helper-predicate",
                "method:class:orders.OtherHelper#predicate()Z",
                json!({
                    "schema":"codeclew-java-compiler-fact/1.0",
                    "declarationKind":"METHOD",
                    "symbolIdentity":"method:class:orders.OtherHelper#predicate()Z",
                    "ownerIdentity":"class:orders.OtherHelper",
                    "name":"predicate",
                    "scope":":main",
                    "jvmDescriptor":"()Z",
                    "modifiers":["STATIC"]
                }),
                Vec::new(),
            ),
            (
                "sibling-helper-type",
                "class:orders.OtherHelperSibling",
                json!({
                    "schema":"codeclew-java-compiler-fact/1.0",
                    "declarationKind":"CLASS",
                    "symbolIdentity":"class:orders.OtherHelperSibling",
                    "ownerIdentity":"class:orders",
                    "name":"OtherHelperSibling",
                    "scope":":main"
                }),
                vec!["sibling-helper-source".to_owned()],
            ),
            (
                "sibling-helper-predicate",
                "method:class:orders.OtherHelperSibling#predicate()Z",
                json!({
                    "schema":"codeclew-java-compiler-fact/1.0",
                    "declarationKind":"METHOD",
                    "symbolIdentity":"method:class:orders.OtherHelperSibling#predicate()Z",
                    "ownerIdentity":"class:orders.OtherHelperSibling",
                    "name":"predicate",
                    "scope":":main",
                    "jvmDescriptor":"()Z",
                    "modifiers":["STATIC"]
                }),
                vec!["sibling-helper-source".to_owned()],
            ),
        ];
        for (id, symbol, normalized, source_ids) in facts {
            let fact_digest = digest(&normalized).unwrap();
            let observation = crate::documentation::model::Observation {
                id: id.into(),
                kind: "SYMBOL".into(),
                service: "orders".into(),
                symbol: symbol.into(),
                normalized,
                digest: fact_digest.clone(),
                source_ids,
            };
            service
                .observations
                .insert(observation.id.clone(), observation.clone());
            work.checked
                .dependencies
                .insert(observation.id.clone(), observation);
            work.influence.insert(id.into(), fact_digest);
            work.handles.insert(
                format!("dependency-{id}"),
                super::super::work::Handle {
                    kind: "DEPENDENCY".into(),
                    id: id.into(),
                },
            );
        }
        work.request.entrypoint = None;
        work.request.context_profile = Some("process-graph-v1".into());
        work.request.root_declaration = Some("endpoint-declaration".into());
        work.request.question = Some("Explain this internal operation and its gaps.".into());
        work.snapshot = Some("snapshot-for-process-packet".into());
        if let Some(shadow) = shadow {
            let source_text = work.checked.services["orders"].sources["endpoint-source"]
                .text
                .clone();
            let root_source = match shadow {
                "parameter" => {
                    source_text.replace("handle(Request request)", "handle(Request OtherHelper)")
                }
                "local" => source_text.replace(
                    "if (OtherHelper.predicate())",
                    "Request OtherHelper = request; if (OtherHelper.predicate())",
                ),
                _ => source_text,
            };
            let source = work
                .checked
                .services
                .get_mut("orders")
                .unwrap()
                .sources
                .get_mut("endpoint-source")
                .unwrap();
            source.text = root_source;
            source.text_digest = crate::canonical::hash_bytes(source.text.as_bytes());
            source.end_line = source.text.lines().count().max(1) as u64;
        }
    }

    fn add_process_context_source(work: &mut Work, id: &str, file: &str, text: &str) {
        let service = work.checked.services.get_mut("orders").unwrap();
        let source = crate::documentation::model::Source {
            id: id.into(),
            service: "orders".into(),
            revision: service.revision.clone(),
            file: file.into(),
            start_line: 1,
            end_line: text.lines().count().max(1) as u64,
            text: text.into(),
            text_digest: crate::canonical::hash_bytes(text.as_bytes()),
            evidence_digest: format!("test-evidence-{id}"),
            authority: "TRANSFORMED_SOURCE".into(),
            occurrence: None,
            url: None,
        };
        service.sources.insert(id.into(), source);
        work.handles.insert(
            format!("source-{id}"),
            super::super::work::Handle {
                kind: "SOURCE".into(),
                id: id.into(),
            },
        );
    }

    fn add_process_context_symbol(
        work: &mut Work,
        id: &str,
        symbol: &str,
        normalized: Value,
        source_ids: &[&str],
    ) {
        let fact_digest = digest(&normalized).unwrap();
        let observation = crate::documentation::model::Observation {
            id: id.into(),
            kind: "SYMBOL".into(),
            service: "orders".into(),
            symbol: symbol.into(),
            normalized,
            digest: fact_digest.clone(),
            source_ids: source_ids
                .iter()
                .map(|source_id| (*source_id).into())
                .collect(),
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

    fn process_constructor_context_work(mapper_source_text: &str) -> Work {
        let mut work = super::super::work::api_contract_tests::endpoint_context_fixture();
        let root_source = "class Controller { Response handle(Request request) { new ConcreteMapper(); this.relay(); return null; } boolean relay() { new ConcreteMapper(); return false; } }";
        let source = work
            .checked
            .services
            .get_mut("orders")
            .unwrap()
            .sources
            .get_mut("endpoint-source")
            .unwrap();
        source.text = root_source.into();
        source.text_digest = crate::canonical::hash_bytes(source.text.as_bytes());
        source.end_line = 1;
        let root = work.checked.services["orders"].observations["endpoint-declaration"]
            .symbol
            .clone();
        add_process_context_symbol(
            &mut work,
            "relay-method",
            "method:class:orders.Controller#relay()Z",
            json!({
                "schema":"codeclew-java-compiler-fact/1.0",
                "declarationKind":"METHOD",
                "symbolIdentity":"method:class:orders.Controller#relay()Z",
                "ownerIdentity":"class:orders.Controller",
                "name":"relay",
                "scope":":main",
                "jvmDescriptor":"()Z"
            }),
            &["endpoint-source"],
        );
        let constructor_flow = crate::documentation::model::Observation {
            id: "flow-concrete-mapper-construction".into(),
            kind: "FLOW".into(),
            service: "orders".into(),
            symbol: root,
            normalized: json!({
                "kind":"CONSTRUCT",
                "target":"method:class:orders.ConcreteMapper#<init>()V",
                "scope":":main",
                "ordinal":0
            }),
            digest: "test-constructor-flow-digest".into(),
            source_ids: vec!["endpoint-source".into()],
        };
        work.checked
            .services
            .get_mut("orders")
            .unwrap()
            .observations
            .insert(constructor_flow.id.clone(), constructor_flow.clone());
        work.checked
            .dependencies
            .insert(constructor_flow.id.clone(), constructor_flow.clone());
        work.influence
            .insert(constructor_flow.id.clone(), constructor_flow.digest.clone());
        work.handles.insert(
            "dependency-flow-concrete-mapper-construction".into(),
            super::super::work::Handle {
                kind: "DEPENDENCY".into(),
                id: constructor_flow.id,
            },
        );

        add_process_context_source(
            &mut work,
            "concrete-mapper-source",
            "src/ConcreteMapper.java",
            mapper_source_text,
        );
        add_process_context_source(
            &mut work,
            "other-helper-context-source",
            "src/OtherHelper.java",
            "class OtherHelper { static boolean predicate() { return true; } }",
        );
        add_process_context_source(
            &mut work,
            "other-config-source",
            "src/OtherConfig.java",
            "class OtherConfig { static final String PREFIX = \"value\"; }",
        );
        add_process_context_source(
            &mut work,
            "duplicate-mapper-source",
            "src/other/ConcreteMapper.java",
            "package other; class ConcreteMapper { boolean unrelated() { return false; } }",
        );
        add_process_context_source(
            &mut work,
            "base-mapper-source",
            "src/BaseMapper.java",
            "abstract class BaseMapper {}",
        );
        add_process_context_symbol(
            &mut work,
            "concrete-mapper-type",
            "class:orders.ConcreteMapper",
            json!({
                "schema":"codeclew-java-compiler-fact/1.0",
                "declarationKind":"CLASS",
                "symbolIdentity":"class:orders.ConcreteMapper",
                "ownerIdentity":"class:orders",
                "name":"ConcreteMapper",
                "scope":":main",
                "superclass":"class:orders.BaseMapper",
                "interfaces":[]
            }),
            &["concrete-mapper-source"],
        );
        add_process_context_symbol(
            &mut work,
            "concrete-mapper-method",
            "method:class:orders.ConcreteMapper#map(Lorders/Request;)Z",
            json!({
                "schema":"codeclew-java-compiler-fact/1.0",
                "declarationKind":"METHOD",
                "symbolIdentity":"method:class:orders.ConcreteMapper#map(Lorders/Request;)Z",
                "ownerIdentity":"class:orders.ConcreteMapper",
                "name":"map",
                "scope":":main",
                "jvmDescriptor":"(Lorders/Request;)Z"
            }),
            &["concrete-mapper-source"],
        );
        add_process_context_symbol(
            &mut work,
            "base-mapper-type",
            "class:orders.BaseMapper",
            json!({
                "schema":"codeclew-java-compiler-fact/1.0",
                "declarationKind":"CLASS",
                "symbolIdentity":"class:orders.BaseMapper",
                "ownerIdentity":"class:orders",
                "name":"BaseMapper",
                "scope":":main"
            }),
            &["base-mapper-source"],
        );
        add_process_context_symbol(
            &mut work,
            "other-helper-context-type",
            "class:orders.OtherHelper",
            json!({
                "schema":"codeclew-java-compiler-fact/1.0",
                "declarationKind":"CLASS",
                "symbolIdentity":"class:orders.OtherHelper",
                "ownerIdentity":"class:orders",
                "name":"OtherHelper",
                "scope":":main"
            }),
            &["other-helper-context-source"],
        );
        add_process_context_symbol(
            &mut work,
            "other-helper-context-method",
            "method:class:orders.OtherHelper#predicate()Z",
            json!({
                "schema":"codeclew-java-compiler-fact/1.0",
                "declarationKind":"METHOD",
                "symbolIdentity":"method:class:orders.OtherHelper#predicate()Z",
                "ownerIdentity":"class:orders.OtherHelper",
                "name":"predicate",
                "scope":":main",
                "jvmDescriptor":"()Z",
                "modifiers":["STATIC"]
            }),
            &["other-helper-context-source"],
        );
        add_process_context_symbol(
            &mut work,
            "other-config-type",
            "class:orders.OtherConfig",
            json!({
                "schema":"codeclew-java-compiler-fact/1.0",
                "declarationKind":"CLASS",
                "symbolIdentity":"class:orders.OtherConfig",
                "ownerIdentity":"class:orders",
                "name":"OtherConfig",
                "scope":":main"
            }),
            &["other-config-source"],
        );
        add_process_context_symbol(
            &mut work,
            "other-config-prefix-field",
            "field:class:orders.OtherConfig#PREFIX:Ljava/lang/String;",
            json!({
                "schema":"codeclew-java-compiler-fact/1.0",
                "declarationKind":"FIELD",
                "symbolIdentity":"field:class:orders.OtherConfig#PREFIX:Ljava/lang/String;",
                "ownerIdentity":"class:orders.OtherConfig",
                "name":"PREFIX",
                "scope":":main",
                "jvmDescriptor":"Ljava/lang/String;",
                "modifiers":["STATIC","FINAL"],
                "sourceTokens":["static","final","String","PREFIX"]
            }),
            &["other-config-source"],
        );
        add_process_context_symbol(
            &mut work,
            "duplicate-concrete-mapper-type",
            "class:orders.other.ConcreteMapper",
            json!({
                "schema":"codeclew-java-compiler-fact/1.0",
                "declarationKind":"CLASS",
                "symbolIdentity":"class:orders.other.ConcreteMapper",
                "ownerIdentity":"class:orders.other",
                "name":"ConcreteMapper",
                "scope":":main"
            }),
            &["duplicate-mapper-source"],
        );
        work.request.entrypoint = None;
        work.request.context_profile = Some("process-graph-v1".into());
        work.request.root_declaration = Some("endpoint-declaration".into());
        work.request.question = Some("Explain this internal process.".into());
        work.snapshot = Some("saved-process-context-snapshot".into());
        work
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
    fn process_graph_profile_builds_an_internal_packet_with_separate_full_audit() {
        let mut work = super::super::work::api_contract_tests::endpoint_context_fixture();
        let root_symbol = work.checked.services["orders"].observations["endpoint-declaration"]
            .symbol
            .clone();
        let retained_target = work.checked.services["orders"].observations["flow-endpoint-service"]
            .normalized["target"]
            .as_str()
            .unwrap()
            .to_owned();
        let extra_flow = crate::documentation::model::Observation {
            id: "flow-endpoint-service-second".into(),
            kind: "FLOW".into(),
            service: "orders".into(),
            symbol: root_symbol,
            normalized: json!({"kind":"CALL","target":retained_target,"scope":":main","ordinal":1}),
            digest: "second-flow-digest".into(),
            source_ids: vec!["endpoint-source".into()],
        };
        work.checked
            .services
            .get_mut("orders")
            .unwrap()
            .observations
            .insert(extra_flow.id.clone(), extra_flow.clone());
        work.checked
            .dependencies
            .insert(extra_flow.id.clone(), extra_flow.clone());
        work.influence
            .insert(extra_flow.id.clone(), extra_flow.digest.clone());
        work.handles.insert(
            "dependency-flow-second".into(),
            super::super::work::Handle {
                kind: "DEPENDENCY".into(),
                id: extra_flow.id,
            },
        );
        work.request.entrypoint = None;
        work.request.context_profile = Some("process-graph-v1".into());
        work.request.root_declaration = Some("endpoint-declaration".into());
        work.request.question = Some("Explain the internal operation and its gaps.".into());
        work.snapshot = Some("snapshot-for-process-packet".into());

        let (packet, audit) = build(&work).unwrap();
        assert_eq!(packet["profile"], "process-graph-v1");
        assert_eq!(
            packet["question"],
            "Explain the internal operation and its gaps."
        );
        assert!(packet.get("endpoint").is_none());
        assert!(packet.get("trigger").is_none());
        assert!(packet.get("exposure").is_none());
        assert_eq!(
            packet["graphAuditBinding"]["snapshot"],
            work.snapshot.unwrap()
        );
        assert!(packet["graphAuditBinding"]["artifactDigest"].is_string());
        assert!(!packet["methods"].as_array().unwrap().is_empty());
        assert!(!packet["methodSources"].as_array().unwrap().is_empty());
        assert!(
            packet["edges"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|edge| edge["authority"] == "RETAINED_FLOW_TARGET")
                .count()
                >= 2,
            "distinct retained callsite observations remain distinct compact edges"
        );
        assert!(audit["processGraph"]["methods"][0]["providerSlots"].is_array());
        assert!(
            audit["records"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["row"]["record"]["id"] == "flow-endpoint-service-second")
        );

        let source_references: Vec<_> = packet["methodSources"]
            .as_array()
            .unwrap()
            .iter()
            .map(|source| source["reference"].as_str().unwrap())
            .collect();
        assert_eq!(
            source_references
                .iter()
                .copied()
                .collect::<BTreeSet<_>>()
                .len(),
            source_references.len(),
            "one author source item per retained source"
        );
        let mut cited = BTreeSet::new();
        collect_evidence(&packet, &mut cited);
        let citations: BTreeSet<_> = packet["citations"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        assert_eq!(citations, cited);
        let packet_text = String::from_utf8(canonical::bytes(&packet).unwrap()).unwrap();
        assert!(!packet_text.contains("providerSlots"));
        assert!(!packet_text.contains("unboundEvents"));
        assert!(!packet_text.contains("\"observation\""));
    }

    #[test]
    fn process_graph_static_helper_candidate_delivers_containing_class_source() {
        let mut work = super::super::work::api_contract_tests::endpoint_context_fixture();
        let root_source = "class Controller { Response handle(Request request) { if (OtherHelper.predicate()) return service.process(request); return null; } }";
        add_process_profile_fields(&mut work, root_source, None);

        let (packet, audit) = build(&work).unwrap();
        let target_identity = "method:class:orders.OtherHelper#predicate()Z";
        assert!(packet["edges"].as_array().unwrap().iter().any(|edge| {
            edge["kind"] == "TYPE_QUALIFIED_CALL"
                && edge["authority"] == "SOURCE_REFERENCE_CANDIDATE"
                && edge["sourceReference"].is_string()
        }));
        let helper_source = packet["methodSources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|source| {
                source["text"]
                    .as_str()
                    .unwrap()
                    .contains("class OtherHelper")
            })
            .unwrap();
        let helper_source_reference = helper_source["reference"].as_str().unwrap();
        assert!(packet["methods"].as_array().unwrap().iter().any(|method| {
            method["symbolIdentity"] == target_identity
                && method["ownerIdentity"] == "class:orders.OtherHelper"
                && method["scope"] == ":main"
                && method["body"]["sourceReference"] == helper_source_reference
        }));
        assert!(!packet["methods"].as_array().unwrap().iter().any(|method| {
            method["symbolIdentity"] == "method:class:orders.OtherHelperSibling#predicate()Z"
        }));
        let helper_method = packet["methods"]
            .as_array()
            .unwrap()
            .iter()
            .find(|method| method["symbolIdentity"] == target_identity)
            .unwrap();
        assert!(packet["edges"].as_array().unwrap().iter().any(|edge| {
            edge["authority"] == "SOURCE_REFERENCE_CANDIDATE"
                && edge["kind"] == "TYPE_QUALIFIED_CALL"
                && edge["targetMethodId"] == helper_method["id"]
        }));
        assert!(
            helper_source["text"]
                .as_str()
                .unwrap()
                .contains("static boolean predicate() { return true; }")
        );
        assert_eq!(
            packet["methodSources"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|source| source["reference"] == helper_source_reference)
                .count(),
            1,
            "containing class text is delivered once even when the method has no own source reference"
        );
        assert!(
            audit["records"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| { row["kind"] == "SOURCE" && row["id"] == "other-helper-source" })
        );

        for shadow in ["parameter", "local"] {
            let mut shadowed = super::super::work::api_contract_tests::endpoint_context_fixture();
            add_process_profile_fields(&mut shadowed, root_source, Some(shadow));
            let (shadow_packet, _) = build(&shadowed).unwrap();
            assert!(
                !shadow_packet["methods"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|method| { method["symbolIdentity"] == target_identity })
            );
            assert!(
                shadow_packet["limitations"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|gap| { gap["code"] == "SOURCE_RECEIVER_SHADOWING_AMBIGUOUS" })
            );
        }
    }

    #[test]
    fn process_constructor_context_resolves_exact_owner_and_referenced_helpers_without_graph_edges()
    {
        let mapper_source = "class ConcreteMapper { boolean map(Request request) { if (OtherHelper.predicate()) return OtherConfig.PREFIX != null; return false; } }";
        let work = process_constructor_context_work(mapper_source);
        let (packet, audit) = build(&work).unwrap();

        let mapper_type = packet["types"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["symbolIdentity"] == "class:orders.ConcreteMapper")
            .unwrap();
        assert_eq!(mapper_type["ownerIdentity"], "class:orders");
        assert_eq!(mapper_type["scope"], ":main");
        assert_eq!(mapper_type["superclass"], "class:orders.BaseMapper");
        let source_contexts = packet["sourceContexts"].as_array().unwrap();
        assert!(source_contexts.iter().any(|context| {
            context["kind"] == "TYPE_SOURCE"
                && context["symbolIdentity"] == "class:orders.ConcreteMapper"
                && context["authority"] == "SOURCE_REFERENCE_CANDIDATE"
                && context["sourceReference"].is_string()
        }));
        assert!(source_contexts.iter().any(|context| {
            context["kind"] == "METHOD_SOURCE"
                && context["symbolIdentity"] == "method:class:orders.OtherHelper#predicate()Z"
                && context["authority"] == "SOURCE_REFERENCE_CANDIDATE"
                && context["referencedFromSourceReference"].is_string()
        }));
        assert!(source_contexts.iter().any(|context| {
            context["kind"] == "STATIC_FIELD_REFERENCE"
                && context["symbolIdentity"]
                    == "field:class:orders.OtherConfig#PREFIX:Ljava/lang/String;"
                && context["sourceReference"].is_null()
                && context["referencedFromSourceReference"].is_string()
        }));
        assert!(packet["fields"].as_array().unwrap().iter().any(|field| {
            field["ownerIdentity"] == "class:orders.OtherConfig" && field["name"] == "PREFIX"
        }));

        let helper_source = packet["methodSources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|source| {
                source["text"]
                    .as_str()
                    .is_some_and(|text| text.contains("class OtherHelper"))
            })
            .unwrap();
        assert!(
            helper_source["text"]
                .as_str()
                .unwrap()
                .contains("static boolean predicate() { return true; }")
        );
        assert_eq!(
            packet["methodSources"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|source| source["reference"] == helper_source["reference"])
                .count(),
            1
        );
        assert!(!packet["methods"].as_array().unwrap().iter().any(|method| {
            method["symbolIdentity"] == "method:class:orders.ConcreteMapper#map(Lorders/Request;)Z"
                || method["symbolIdentity"] == "method:class:orders.OtherHelper#predicate()Z"
        }));
        assert!(packet["edges"].as_array().unwrap().iter().all(|edge| {
            edge["targetIdentity"] != "method:class:orders.ConcreteMapper#map(Lorders/Request;)Z"
                && edge["targetIdentity"] != "method:class:orders.OtherHelper#predicate()Z"
        }));
        assert!(packet["limitations"].as_array().unwrap().iter().any(|gap| {
            gap["code"] == "PROCESS_CONSTRUCTED_TYPE_AMBIGUOUS"
                && gap["examples"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|example| example["from"] == "method:class:orders.Controller#relay()Z")
        }));
        assert!(audit["records"].as_array().unwrap().iter().any(|record| {
            record["kind"] == "DEPENDENCY"
                && record["row"]["record"]["normalized"]["symbolIdentity"]
                    == "class:orders.ConcreteMapper"
        }));
    }

    #[test]
    fn process_context_does_not_resolve_shadowed_static_type_or_deliver_missing_helper_body() {
        for mapper_source in [
            "class ConcreteMapper { boolean map(Request OtherConfig) { return OtherConfig.PREFIX != null; } }",
            "class ConcreteMapper { boolean map(Request request) { Request OtherConfig = request; return OtherConfig.PREFIX != null; } }",
        ] {
            let work = process_constructor_context_work(mapper_source);
            let (packet, _) = build(&work).unwrap();
            assert!(!packet["fields"].as_array().unwrap().iter().any(|field| {
                field["ownerIdentity"] == "class:orders.OtherConfig" && field["name"] == "PREFIX"
            }));
            assert!(
                packet["limitations"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|gap| { gap["code"] == "PROCESS_QUALIFIED_FIELD_TYPE_SHADOWED" })
            );
        }

        let mut missing = process_constructor_context_work(
            "class ConcreteMapper { boolean map(Request request) { return OtherHelper.predicate(); } }",
        );
        missing
            .checked
            .services
            .get_mut("orders")
            .unwrap()
            .sources
            .get_mut("other-helper-context-source")
            .unwrap()
            .text = "class OtherHelper {}".into();
        let source = missing
            .checked
            .services
            .get_mut("orders")
            .unwrap()
            .sources
            .get_mut("other-helper-context-source")
            .unwrap();
        source.text_digest = crate::canonical::hash_bytes(source.text.as_bytes());
        let (packet, _) = build(&missing).unwrap();
        assert!(packet["limitations"].as_array().unwrap().iter().any(|gap| {
            gap["code"] == "SOURCE_CONTEXT_METHOD_BODY_UNAVAILABLE"
                || gap["code"] == "SOURCE_CONTEXT_METHOD_BODY_AMBIGUOUS"
        }));
        assert!(
            packet["sourceContexts"]
                .as_array()
                .unwrap()
                .iter()
                .any(|context| {
                    context["kind"] == "METHOD_SOURCE"
                        && context["symbolIdentity"]
                            == "method:class:orders.OtherHelper#predicate()Z"
                        && context["sourceReference"].is_null()
                })
        );
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
            "receiver FIELD evidence must remain selected for its source-based call candidate"
        );
        assert!(
            profile["record"]["referencedOwnerFields"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|group| group["fieldReferences"].as_array().unwrap())
                .any(|reference| reference == receiver_reference)
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

    #[test]
    fn authoring_contract_1_4_delivers_selected_owner_field_declarations_separately_from_constants()
    {
        let mut work = super::super::work::api_contract_tests::endpoint_context_fixture();
        work.request.authoring_contract =
            Some(super::super::operation_answer::AUTHORING_CONTRACT.into());
        {
            let service = work.checked.services.get_mut("orders").unwrap();
            let source = service.sources.get_mut("service-source").unwrap();
            source.text = source.text.replace(
                "    try {\n",
                "    Object guardSnapshot = this.guardCodes;\n    int timeoutSnapshot = timeoutMillis;\n    try {\n",
            );
            let class_end = source.text.rfind('}').unwrap();
            source.text.insert_str(
                class_end,
                "  private final Set<String> guardCodes = new HashSet<String>();\n  private static int timeoutMillis = 250;\n",
            );
            source
                .text
                .push_str("class OtherService { static int timeoutMillis = 900; }\n");
            source.text_digest = canonical::hash_bytes(source.text.as_bytes());
        }
        add_owner_field(
            &mut work,
            "guard-field",
            "class:orders.Service",
            "guardCodes",
            "Ljava/util/Set;",
            &["PRIVATE", "FINAL"],
            &[
                "private",
                "final",
                "Set",
                "<",
                "String",
                ">",
                "guardCodes",
                "=",
                "new",
                "HashSet",
                "<",
                "String",
                ">",
                "(",
                ")",
                ";",
            ],
        );
        add_owner_field(
            &mut work,
            "timeout-field",
            "class:orders.Service",
            "timeoutMillis",
            "I",
            &["PRIVATE", "STATIC"],
            &["private", "static", "int", "timeoutMillis", "=", "250", ";"],
        );
        add_owner_field(
            &mut work,
            "other-timeout",
            "class:orders.OtherService",
            "timeoutMillis",
            "I",
            &["STATIC"],
            &["static", "int", "timeoutMillis", "=", "900", ";"],
        );

        let (packet, audit) = build(&work).unwrap();
        if let Some(path) = std::env::var_os("CODECLEW_TEST_ENDPOINT_PACKET_PATH") {
            std::fs::write(path, serde_json::to_vec_pretty(&packet).unwrap()).unwrap();
        }
        let (repeat_packet, repeat_audit) = build(&work).unwrap();
        assert_eq!(packet, repeat_packet);
        assert_eq!(audit, repeat_audit);
        let fields = packet["fields"].as_array().unwrap();
        assert_eq!(fields.len(), 3);
        let field = |name: &str| fields.iter().find(|field| field["name"] == name).unwrap();

        let guard = field("guardCodes");
        assert_eq!(guard["ownerIdentity"], "class:orders.Service");
        assert_eq!(guard["typeDescriptor"], "Ljava/util/Set;");
        assert_eq!(guard["modifiers"], json!(["PRIVATE", "FINAL"]));
        assert_eq!(
            guard["sourceTokens"],
            json!([
                "private",
                "final",
                "Set",
                "<",
                "String",
                ">",
                "guardCodes",
                "=",
                "new",
                "HashSet",
                "<",
                "String",
                ">",
                "(",
                ")",
                ";"
            ])
        );
        assert_eq!(guard["evidence"], json!(["dependency-guard-field"]));

        let timeout = field("timeoutMillis");
        assert_eq!(timeout["ownerIdentity"], "class:orders.Service");
        assert_eq!(timeout["typeDescriptor"], "I");
        assert_eq!(timeout["modifiers"], json!(["PRIVATE", "STATIC"]));
        assert_eq!(
            timeout["sourceTokens"],
            json!(["private", "static", "int", "timeoutMillis", "=", "250", ";"])
        );
        assert_eq!(timeout["evidence"], json!(["dependency-timeout-field"]));
        assert!(
            !fields
                .iter()
                .any(|field| field["ownerIdentity"] == "class:orders.OtherService")
        );

        for (id, field) in [("guard-field", guard), ("timeout-field", timeout)] {
            let evidence = field["evidence"][0].as_str().unwrap();
            let record = audit["records"]
                .as_array()
                .unwrap()
                .iter()
                .find(|record| record["id"] == id)
                .unwrap();
            assert_eq!(record["label"], evidence);
            assert_eq!(record["deliveredToAuthor"], false);
            assert_eq!(
                record["row"]["record"]["normalized"]["sourceTokens"],
                field["sourceTokens"]
            );
            assert_eq!(
                packet["citations"][evidence],
                "captured owner field declaration"
            );
        }
        assert_eq!(audit["packetDigest"], packet["packetDigest"]);

        let constants = packet["constants"].as_array().unwrap();
        assert_eq!(constants.len(), 1);
        assert_eq!(constants[0]["name"], "DEFAULT_CODE");
        assert_eq!(constants[0]["modifiers"], json!(["STATIC", "FINAL"]));
        assert!(
            packet["interpretationLimits"]
                .as_array()
                .unwrap()
                .iter()
                .any(|limit| {
                    limit.as_str().is_some_and(|text| {
                        text.contains("final does not establish deep immutability")
                    })
                })
        );
        assert_eq!(
            packet["citations"]["dependency-guard-field"],
            "captured owner field declaration"
        );
        assert_eq!(
            packet["citations"]["dependency-timeout-field"],
            "captured owner field declaration"
        );
        assert!(work.checked.dependencies.contains_key("other-timeout"));
        assert!(
            audit["records"]
                .as_array()
                .unwrap()
                .iter()
                .all(|record| record["id"] != "other-timeout")
        );
    }

    fn source_fixture(
        id: &str,
        file: &str,
        start_line: u64,
        end_line: u64,
        text: &str,
        occurrence: Option<crate::documentation::model::SourceOccurrence>,
    ) -> Source {
        Source {
            id: id.into(),
            service: "svc".into(),
            revision: "rev".into(),
            file: file.into(),
            start_line,
            end_line,
            text: text.into(),
            text_digest: crate::canonical::hash_bytes(text.as_bytes()),
            evidence_digest: format!("evidence-{id}"),
            authority: "TRANSFORMED_SOURCE".into(),
            occurrence,
            url: None,
        }
    }

    #[test]
    fn process_source_ranges_require_exact_unique_location_and_matching_provenance() {
        let parent_text = "class C {\n  static String value = \"x\";\n}\n";
        let part_text = "  static String value = \"x\";";
        let parent = source_fixture("parent", "src/C.java", 20, 22, parent_text, None);
        let part = source_fixture("part", "src/C.java", 21, 21, part_text, None);
        let start = parent_text.find(part_text).unwrap();
        assert_eq!(
            exact_source_range(&parent, &part),
            Some((start, start + part_text.len()))
        );

        let repeated_text = "class C { static String value = \"x\"; static String value = \"x\"; }";
        let repeated = source_fixture("repeat", "src/C.java", 20, 20, repeated_text, None);
        let same_line_part = source_fixture(
            "part",
            "src/C.java",
            20,
            20,
            "static String value = \"x\";",
            None,
        );
        assert_eq!(exact_source_range(&repeated, &same_line_part), None);

        let other_file = source_fixture("other", "src/Other.java", 21, 21, part_text, None);
        assert_eq!(exact_source_range(&parent, &other_file), None);

        let snapshot = "snapshot-a";
        let blob = "blob-a";
        let mut parent_with_occurrence = parent.clone();
        parent_with_occurrence.occurrence = Some(crate::documentation::model::SourceOccurrence {
            snapshot: snapshot.into(),
            blob: blob.into(),
            start_byte: 0,
            end_byte: parent_text.len(),
        });
        let mut part_with_occurrence = part.clone();
        part_with_occurrence.occurrence = Some(crate::documentation::model::SourceOccurrence {
            snapshot: snapshot.into(),
            blob: blob.into(),
            start_byte: start,
            end_byte: start + part_text.len(),
        });
        assert_eq!(
            exact_source_range(&parent_with_occurrence, &part_with_occurrence),
            Some((start, start + part_text.len()))
        );
        part_with_occurrence.occurrence.as_mut().unwrap().blob = "blob-b".into();
        assert_eq!(
            exact_source_range(&parent_with_occurrence, &part_with_occurrence),
            None
        );
    }

    #[test]
    fn process_packet_aliases_method_source_to_its_exact_owner_class_range() {
        let parent_text = "class Widget {\n  void map() {\n    Helpers.value();\n  }\n}\n";
        let part_text = "  void map() {\n    Helpers.value();\n  }";
        let start = parent_text.find(part_text).unwrap();
        let snapshot = "snapshot-a";
        let blob = "blob-widget";
        let parent = source_fixture(
            "class-source",
            "src/Widget.java",
            20,
            24,
            parent_text,
            Some(crate::documentation::model::SourceOccurrence {
                snapshot: snapshot.into(),
                blob: blob.into(),
                start_byte: 0,
                end_byte: parent_text.len(),
            }),
        );
        let part = source_fixture(
            "method-source",
            "src/Widget.java",
            21,
            23,
            part_text,
            Some(crate::documentation::model::SourceOccurrence {
                snapshot: snapshot.into(),
                blob: blob.into(),
                start_byte: start,
                end_byte: start + part_text.len(),
            }),
        );
        let selected = BTreeMap::from([
            (
                "source-class".into(),
                (
                    "label-class".into(),
                    json!({"kind":"SOURCE","record":parent}),
                ),
            ),
            (
                "source-method".into(),
                (
                    "label-method".into(),
                    json!({"kind":"SOURCE","record":part}),
                ),
            ),
            (
                "type-widget".into(),
                (
                    "label-type".into(),
                    json!({"kind":"DEPENDENCY","record":{"normalized":{"symbolIdentity":"class:svc.Widget","scope":":main"}}}),
                ),
            ),
        ]);
        let mut sources = vec![
            json!({"reference":"source-method","authority":"TRANSFORMED_SOURCE","text":part_text,"evidence":["label-method"]}),
            json!({"reference":"source-class","authority":"TRANSFORMED_SOURCE","text":parent_text,"evidence":["label-class"],"contextFor":"type-widget","contextReferences":["type-widget"]}),
        ];
        let mut methods = vec![json!({
            "ownerIdentity":"class:svc.Widget",
            "scope":":main",
            "body":{"sourceReference":"source-method","startByte":0,"endByte":part_text.len(),"evidence":["label-method"]}
        })];
        let mut edges = vec![json!({"sourceReference":"source-method"})];

        coalesce_process_method_sources(&mut sources, &mut methods, &mut edges, &selected).unwrap();

        assert_eq!(sources.len(), 1);
        let alias = &sources[0]["sourceAliases"][0];
        assert_eq!(alias["reference"], "source-method");
        assert_eq!(alias["startByte"], start);
        assert_eq!(alias["endByte"], start + part_text.len());
        assert_eq!(alias["evidence"], json!(["label-method"]));
        assert_eq!(methods[0]["body"]["sourceReference"], "source-class");
        assert_eq!(
            &parent_text[methods[0]["body"]["startByte"].as_u64().unwrap() as usize
                ..methods[0]["body"]["endByte"].as_u64().unwrap() as usize],
            part_text
        );
        assert_eq!(edges[0]["sourceReference"], "source-class");

        let mut other_owner_methods = vec![json!({
            "ownerIdentity":"class:svc.OtherWidget",
            "scope":":main",
            "body":{"sourceReference":"source-method","startByte":0,"endByte":part_text.len(),"evidence":["label-method"]}
        })];
        let mut untouched_sources = vec![
            json!({"reference":"source-method","authority":"TRANSFORMED_SOURCE","text":part_text,"evidence":["label-method"]}),
            json!({"reference":"source-class","authority":"TRANSFORMED_SOURCE","text":parent_text,"evidence":["label-class"],"contextFor":"type-widget","contextReferences":["type-widget"]}),
        ];
        coalesce_process_method_sources(
            &mut untouched_sources,
            &mut other_owner_methods,
            &mut [],
            &selected,
        )
        .unwrap();
        assert_eq!(untouched_sources.len(), 2);
        assert_eq!(
            other_owner_methods[0]["body"]["sourceReference"],
            "source-method"
        );
    }
}
