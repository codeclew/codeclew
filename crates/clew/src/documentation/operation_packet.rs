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
                "typeDescriptor":normalized["typeDescriptor"],
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
        let authority =
            if record["kind"] == "CALL_RELATION" && normalized["resolution"] == "COMPILER_EXACT" {
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
        cite(&mut citations, label, "retained provider call evidence");
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
        call_edges.push(json!({
            "from":from,
            "toNode":to_node,
            "targetIdentity":target_identity,
            "kind":candidate["kind"],
            "scope":call_nodes.iter().find(|node| node["id"] == from).map(|node|node["scope"].clone()).unwrap_or(Value::Null),
            "authority":"SOURCE_REFERENCE_CANDIDATE",
            "evidence":evidence
        }));
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
                "typeDescriptor":normalized["typeDescriptor"],
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
            "Method names do not establish runtime behavior or side effects."
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
}
