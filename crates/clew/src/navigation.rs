use crate::canonical;
use crate::error::{ClewError, ErrorCode};
use crate::session::ContextObject;
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub const NAV_QUERY_SCHEMA: &str = "codeclew-nav-query/2.0";
pub const NAV_EXPAND_SCHEMA: &str = "codeclew-nav-expand/2.0";
pub const NAV_RESULT_SCHEMA: &str = "codeclew-navigation-result/2.1";
pub const NAV_DELTA_SCHEMA: &str = "codeclew-navigation-delta/2.1";
pub const NAV_DETAIL_SCHEMA: &str = "codeclew-navigation-detail/1.1";
pub const NAV_AGENT_CARD_SCHEMA: &str = "codeclew-navigation-agent-card/1.1";
pub const NAV_ACTION_SCHEMA: &str = "codeclew-navigation-actions/1.0";
pub const NAV_DECISION_AUTHORITY_SCHEMA: &str = "codeclew-navigation-decision-authority/1.0";
pub const NAV_QUERY_INTENT: &str = "NAVIGATION_QUERY";
pub const MAX_NAV_STDOUT_BYTES: usize = 64 * 1024;
const MAX_NAV_CANDIDATES: usize = 3;
const MAX_TERM_ANCHORS: usize = 4;
const MAX_REFERENCE_FOLLOW_TERMS: usize = 3;
const MAX_REFERENCE_MATCHES_PER_TERM: usize = 3;
const MAX_REFERENCE_FOLLOW_SOURCE_BYTES: usize = 16 * 1024;
const MAX_REFERENCE_OBSERVATIONS_PER_TERM: usize = 4;
const MAX_REFERENCE_PATH_BYTES: usize = 512;
const MAX_DECISION_SOURCE_DECLARATIONS: usize = 3;
const MAX_DECISION_SOURCE_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum NavigationFacet {
    Callers,
    Callees,
    Tests,
}

pub fn query(context: &ContextObject, facets: &[NavigationFacet]) -> Result<Value, ClewError> {
    query_with_decision_identifier(context, facets, None)
}

pub fn query_with_decision_identifier(
    context: &ContextObject,
    facets: &[NavigationFacet],
    decision_identifier: Option<&str>,
) -> Result<Value, ClewError> {
    let retained = retained_context(context)?;
    let result = assemble_with_decision_identifier(
        &context.session_id,
        &context.context_id,
        &context.evidence_digest,
        &context.intent,
        &context.terms,
        retained,
        retained_query_truncated(context),
        facets,
        decision_identifier,
    )?;
    validate_stdout(&result)?;
    Ok(result)
}

pub fn agent_card(result: &Value) -> Result<Value, ClewError> {
    if result.get("schema").and_then(Value::as_str) != Some(NAV_RESULT_SCHEMA) {
        return Err(invalid("agent card input is not a navigation result"));
    }
    let candidates = result
        .get("candidates")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("navigation result has no candidate array"))?
        .iter()
        .map(compact_agent_candidate)
        .collect::<Result<Vec<_>, _>>()?;
    let decision_source = result
        .get("decisionSource")
        .ok_or_else(|| invalid("navigation result has no decision source"))?;
    let decision_authority = result
        .get("decisionAuthority")
        .ok_or_else(|| invalid("navigation result has no decision authority"))?;
    validate_agent_card_decision_contract(result, decision_authority, decision_source)?;
    let compact_source = compact_decision_source(decision_source)?;
    let supporting_anchors = result
        .get("termAnchors")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("navigation result has no term anchors"))?
        .iter()
        .filter(|anchor| !anchor_is_in_decision_source(anchor, decision_source))
        .cloned()
        .collect::<Vec<_>>();
    let terms = result
        .get("terms")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("navigation result has no term array"))?;
    let candidate_count = result
        .get("candidateCount")
        .and_then(Value::as_object)
        .ok_or_else(|| invalid("navigation result has no candidate count"))?;
    let completeness = result
        .get("completeness")
        .and_then(Value::as_object)
        .ok_or_else(|| invalid("navigation result has no completeness"))?;
    let truncated = result
        .get("truncated")
        .and_then(Value::as_bool)
        .ok_or_else(|| invalid("navigation result has no truncated flag"))?;
    let query_coverage_truncated = result
        .get("queryCoverageTruncated")
        .and_then(Value::as_bool)
        .ok_or_else(|| invalid("navigation result has no query coverage truncation flag"))?;
    let candidate_list_truncated = result
        .get("candidateListTruncated")
        .and_then(Value::as_bool)
        .ok_or_else(|| invalid("navigation result has no candidate list truncation flag"))?;
    let next_action = result
        .get("nextAction")
        .and_then(Value::as_object)
        .ok_or_else(|| invalid("navigation result has no next action"))?;
    let mut card = json!({
        "schema":NAV_AGENT_CARD_SCHEMA,
        "sessionId":required_value_string(result, "sessionId")?,
        "contextId":required_value_string(result, "contextId")?,
        "evidenceDigest":required_value_string(result, "evidenceDigest")?,
        "terms":terms,
        "candidates":candidates,
        "candidateCount":candidate_count,
        "decisionAuthority":decision_authority,
        "decisionSource":compact_source,
        "supportingAnchors":supporting_anchors,
        "completeness":completeness,
        "queryCoverageTruncated":query_coverage_truncated,
        "candidateListTruncated":candidate_list_truncated,
        "truncated":truncated,
        "nextAction":next_action,
    });
    if let Some(next_actions) = result.get("nextActions") {
        card.as_object_mut()
            .expect("agent card is an object")
            .insert("nextActions".into(), next_actions.clone());
    }
    if let Some(reference_follow) = result.get("referenceFollow") {
        card.as_object_mut()
            .expect("agent card is an object")
            .insert("referenceFollow".into(), reference_follow.clone());
    }
    Ok(card)
}

fn validate_agent_card_decision_contract(
    result: &Value,
    authority: &Value,
    decision_source: &Value,
) -> Result<(), ClewError> {
    if authority.get("schema").and_then(Value::as_str) != Some(NAV_DECISION_AUTHORITY_SCHEMA) {
        return Err(invalid(
            "navigation decision authority has an invalid schema",
        ));
    }
    match authority.get("status").and_then(Value::as_str) {
        Some("ABSTAIN") => {
            if decision_source.get("status").and_then(Value::as_str) != Some("UNAVAILABLE")
                || decision_source.get("reason").and_then(Value::as_str)
                    != Some("DECISION_AUTHORITY_ABSTAINED")
            {
                return Err(invalid(
                    "abstained navigation decision must not carry a decision source",
                ));
            }
            if let Some(reference_follow) = result.get("referenceFollow")
                && (reference_follow.get("status").and_then(Value::as_str) != Some("UNAVAILABLE")
                    || reference_follow.get("reason").and_then(Value::as_str)
                        != Some("DECISION_AUTHORITY_ABSTAINED"))
            {
                return Err(invalid(
                    "abstained navigation decision must not carry reference follow evidence",
                ));
            }
        }
        Some("SUPPORTED") => {
            let candidate_id = required_value_string(authority, "candidateId")?;
            if decision_source.get("candidateId").and_then(Value::as_str) != Some(candidate_id) {
                return Err(invalid(
                    "supported navigation decision source does not match its authority",
                ));
            }
        }
        _ => {
            return Err(invalid(
                "navigation decision authority has an invalid status",
            ));
        }
    }
    Ok(())
}

fn compact_agent_candidate(candidate: &Value) -> Result<Value, ClewError> {
    let location = candidate
        .get("location")
        .ok_or_else(|| invalid("navigation candidate has no location"))?;
    let preview = candidate
        .get("preview")
        .ok_or_else(|| invalid("navigation candidate has no preview"))?;
    let preview = match preview.get("status").and_then(Value::as_str) {
        Some("EXACT") => json!({
            "status":"EXACT",
            "authority":required_value_string(preview, "authority")?,
            "contentDigest":required_value_string(preview, "contentDigest")?,
            "startLine":required_value_u64(preview, "startLine")?,
            "endLine":required_value_u64(preview, "endLine")?,
            "text":required_value_string(preview, "text")?,
            "truncated":preview
                .get("truncated")
                .and_then(Value::as_bool)
                .ok_or_else(|| invalid("exact navigation preview has no truncated flag"))?,
        }),
        Some("UNAVAILABLE") => json!({
            "status":"UNAVAILABLE",
            "reason":required_value_string(preview, "reason")?,
        }),
        _ => return Err(invalid("navigation preview has an invalid status")),
    };
    let mut compact = json!({
        "candidateId":required_value_string(candidate, "candidateId")?,
        "displayName":required_value_string(candidate, "displayName")?,
        "declarationKind":required_value_string(candidate, "declarationKind")?,
        "location":{
            "file":required_value_string(location, "file")?,
            "startLine":required_value_u64(location, "startLine")?,
            "endLine":required_value_u64(location, "endLine")?,
        },
        "preview":preview,
    });
    for field in ["displayNameTruncated", "declarationKindTruncated"] {
        if let Some(value) = candidate.get(field) {
            compact
                .as_object_mut()
                .expect("compact candidate is an object")
                .insert(field.into(), value.clone());
        }
    }
    if let Some(value) = location.get("fileTruncated") {
        compact
            .pointer_mut("/location")
            .expect("compact candidate has a location")
            .as_object_mut()
            .expect("compact candidate location is an object")
            .insert("fileTruncated".into(), value.clone());
    }
    Ok(compact)
}

fn compact_decision_source(decision: &Value) -> Result<Value, ClewError> {
    let Some(source) = decision.get("source") else {
        return match decision.get("status").and_then(Value::as_str) {
            Some("UNAVAILABLE") => Ok(json!({
                "status":"UNAVAILABLE",
                "reason":required_value_string(decision, "reason")?,
            })),
            _ => Err(invalid("decision source has an invalid status")),
        };
    };
    match source.get("status").and_then(Value::as_str) {
        Some("SUPPORTED") => {
            let bindings = decision
                .get("sourceBindings")
                .and_then(Value::as_array)
                .ok_or_else(|| invalid("supported decision source has no bindings"))?
                .iter()
                .map(|binding| {
                    Ok(json!({
                        "candidateId":required_value_string(binding, "candidateId")?,
                        "displayName":required_value_string(binding, "displayName")?,
                        "declarationStartLine":required_value_u64(binding, "declarationStartLine")?,
                        "declarationEndLine":required_value_u64(binding, "declarationEndLine")?,
                        "windowIndex":required_value_u64(binding, "windowIndex")?,
                    }))
                })
                .collect::<Result<Vec<_>, ClewError>>()?;
            let binding_count = decision
                .get("sourceBindingCount")
                .ok_or_else(|| invalid("supported decision source has no binding count"))?;
            let returned_candidate_ids = bindings
                .iter()
                .map(|binding| required_value_string(binding, "candidateId"))
                .collect::<Result<Vec<_>, _>>()?;
            let windows = source
                .get("windows")
                .and_then(Value::as_array)
                .ok_or_else(|| invalid("supported decision source has no windows"))?;
            Ok(json!({
                "candidateId":required_value_string(decision, "candidateId")?,
                "selectionAuthority":required_value_string(decision, "selectionAuthority")?,
                "sourceBindingCount":binding_count,
                "sourceBindings":bindings,
                "sourceDelivery":{
                    "status":"RETURNED",
                    "candidateIds":returned_candidate_ids,
                    "reuse":"CURRENT_RESULT",
                    "repeatSameRequest":false,
                },
                "source":{
                    "status":"SUPPORTED",
                    "authority":required_value_string(source, "authority")?,
                    "fileId":required_value_string(source, "fileId")?,
                    "contentDigest":source
                        .pointer("/contentRef/digest")
                        .and_then(Value::as_str)
                        .ok_or_else(|| invalid("supported decision source has no content digest"))?,
                    "completeFile":source
                        .get("completeFile")
                        .and_then(Value::as_bool)
                        .ok_or_else(|| invalid("supported decision source has no complete-file flag"))?,
                    "truncated":source
                        .get("truncated")
                        .and_then(Value::as_bool)
                        .ok_or_else(|| invalid("supported decision source has no truncated flag"))?,
                    "windows":windows,
                },
            }))
        }
        Some(status @ ("UNSUPPORTED" | "UNAVAILABLE")) => Ok(json!({
            "candidateId":required_value_string(decision, "candidateId")?,
            "sourceDelivery":{
                "status":status,
                "reason":required_value_string(source, "reason")?,
                "repeatSameRequest":false,
            },
            "source":{
                "status":status,
                "reason":required_value_string(source, "reason")?,
            },
        })),
        _ => Err(invalid("decision source has an invalid status")),
    }
}

fn anchor_is_in_decision_source(anchor: &Value, decision: &Value) -> bool {
    let Some(file) = anchor.get("file").and_then(Value::as_str) else {
        return false;
    };
    let Some(line) = anchor.get("line").and_then(Value::as_u64) else {
        return false;
    };
    if decision.pointer("/source/fileId").and_then(Value::as_str) != Some(file) {
        return false;
    }
    decision
        .pointer("/source/windows")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .any(|window| {
            window
                .get("startLine")
                .and_then(Value::as_u64)
                .is_some_and(|start| start <= line)
                && window
                    .get("endLine")
                    .and_then(Value::as_u64)
                    .is_some_and(|end| line <= end)
        })
}

pub fn expand_delta(
    parent: &ContextObject,
    child: &ContextObject,
    requested_terms: &[String],
    facets: &[NavigationFacet],
) -> Result<Value, ClewError> {
    expand_delta_with_decision_identifier(parent, child, requested_terms, facets, None)
}

pub fn expand_delta_with_decision_identifier(
    parent: &ContextObject,
    child: &ContextObject,
    requested_terms: &[String],
    facets: &[NavigationFacet],
    decision_identifier: Option<&str>,
) -> Result<Value, ClewError> {
    if requested_terms.is_empty()
        || parent.session_id != child.session_id
        || child.parent_context_id.as_deref() != Some(parent.context_id.as_str())
    {
        return Err(invalid(
            "navigation delta requires a direct child context in the same session",
        ));
    }
    let parent_view = assemble(
        &parent.session_id,
        &parent.context_id,
        &parent.evidence_digest,
        &parent.intent,
        &parent.terms,
        retained_context(parent)?,
        retained_query_truncated(parent),
        &[],
    )?;
    let child_view = assemble_with_decision_identifier(
        &child.session_id,
        &child.context_id,
        &child.evidence_digest,
        &child.intent,
        &child.terms,
        retained_context(child)?,
        retained_query_truncated(child),
        facets,
        decision_identifier,
    )?;
    let parent_candidates = candidate_map(&parent_view)?;
    let child_candidates = candidate_map(&child_view)?;
    let mut upserts = Vec::new();
    let mut unchanged_count = 0usize;
    for (key, candidate) in &child_candidates {
        match parent_candidates.get(key) {
            None => upserts.push(candidate_upsert(candidate, "ADDED")?),
            Some(previous) if *previous == *candidate => unchanged_count += 1,
            Some(_) => upserts.push(candidate_upsert(candidate, "UPDATED")?),
        }
    }
    let removals = parent_candidates
        .keys()
        .filter(|key| !child_candidates.contains_key(*key))
        .map(|(compilation, fact_key)| json!({"compilation":compilation,"factKey":fact_key}))
        .collect::<Vec<_>>();
    let candidate_order = child_view["candidates"]
        .as_array()
        .ok_or_else(|| invalid("navigation result has no candidate array"))?
        .iter()
        .map(|candidate| {
            let (compilation, fact_key) = candidate_key(candidate)?;
            Ok(json!({"compilation":compilation,"factKey":fact_key}))
        })
        .collect::<Result<Vec<_>, ClewError>>()?;
    let result = json!({
        "schema":NAV_DELTA_SCHEMA,
        "sessionId":child.session_id,
        "parentContextId":parent.context_id,
        "parentEvidenceDigest":parent.evidence_digest,
        "contextId":child.context_id,
        "evidenceDigest":child.evidence_digest,
        "intent":child.intent,
        "requestedTerms":requested_terms,
        "contextTerms":child.terms,
        "candidateDelta":{
            "upserts":upserts,
            "removals":removals,
            "unchangedCount":unchanged_count,
            "candidateOrder":candidate_order,
        },
        "termAnchors":child_view["termAnchors"],
        "facets":child_view["facets"],
        "decisionAuthority":child_view["decisionAuthority"],
        "completeness":child_view["completeness"],
        "queryCoverageTruncated":child_view["queryCoverageTruncated"],
        "candidateListTruncated":child_view["candidateListTruncated"],
        "truncated":child_view["truncated"],
    });
    validate_stdout(&result)?;
    Ok(result)
}

pub fn detail(
    context: &ContextObject,
    candidate_ids: &[String],
    include_source: bool,
    facets: &[NavigationFacet],
) -> Result<Value, ClewError> {
    if candidate_ids.is_empty() || candidate_ids.len() > MAX_NAV_CANDIDATES {
        return Err(invalid(
            "navigation detail requires one to three candidates",
        ));
    }
    let unique = candidate_ids.iter().collect::<BTreeSet<_>>();
    if unique.len() != candidate_ids.len() {
        return Err(invalid("navigation detail candidates are duplicated"));
    }
    let retained = retained_context(context)?;
    let matches = retained
        .get("matches")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("retained navigation context has no match array"))?;
    let sources = retained
        .get("sources")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("retained navigation context has no source array"))?;
    let requested_facets = facets.iter().copied().collect::<BTreeSet<_>>();
    let selected = candidate_ids
        .iter()
        .map(|candidate_id| {
            selected_candidate_detail(
                matches,
                sources,
                candidate_id,
                include_source,
                &requested_facets,
            )
        })
        .collect::<Result<Vec<_>, ClewError>>()?;
    let result = json!({
        "schema":NAV_DETAIL_SCHEMA,
        "sessionId":context.session_id,
        "contextId":context.context_id,
        "evidenceDigest":context.evidence_digest,
        "candidates":selected,
        "contextCompleteness":retained.get("completeness"),
        "truncated":retained_query_truncated(context),
    });
    validate_stdout(&result)?;
    Ok(result)
}

pub fn detail_by_exact_file_term(
    context: &ContextObject,
    file: &str,
    term: &str,
    include_source: bool,
) -> Result<Value, ClewError> {
    detail_by_exact_file_terms(context, file, &[term.to_owned()], include_source)
}

pub fn detail_by_exact_file_terms(
    context: &ContextObject,
    file: &str,
    terms: &[String],
    include_source: bool,
) -> Result<Value, ClewError> {
    validate_file_selector(file)?;
    if !(1..=MAX_NAV_CANDIDATES).contains(&terms.len())
        || terms.iter().any(String::is_empty)
        || terms.iter().collect::<BTreeSet<_>>().len() != terms.len()
    {
        return Err(invalid(
            "exact navigation requires one to three unique terms",
        ));
    }
    let retained = retained_context(context)?;
    let matches = retained
        .get("matches")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("retained navigation context has no match array"))?;
    let mut candidate_ids = Vec::new();
    for term in terms {
        let mut term_candidate_ids = Vec::new();
        for matched in matches {
            let Some(payload) = matched.get("payload").and_then(Value::as_object) else {
                continue;
            };
            if !is_declaration(payload)
                || payload.get("file").and_then(Value::as_str) != Some(file)
                || !exact_candidate_name_matches(payload, term)
            {
                continue;
            }
            term_candidate_ids.push(candidate_handle(
                required_string(matched, "compilation")?,
                required_string(matched, "factKey")?,
            )?);
        }
        match term_candidate_ids.len() {
            0 => {
                return Err(ClewError::new(
                    ErrorCode::SymbolNotFound,
                    format!("no exact declaration matches file and term {term}"),
                ));
            }
            1 => candidate_ids.push(term_candidate_ids.remove(0)),
            _ => {
                return Err(ClewError::new(
                    ErrorCode::AmbiguousSymbol,
                    format!("multiple exact declarations match file and term {term}"),
                ));
            }
        }
    }
    detail(context, &candidate_ids, include_source, &[])
}

pub fn source_by_candidate(
    context: &ContextObject,
    candidate_id: &str,
) -> Result<Value, ClewError> {
    let retained = retained_context(context)?;
    let matches = retained
        .get("matches")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("retained navigation context has no match array"))?;
    let sources = retained
        .get("sources")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("retained navigation context has no source array"))?;
    let payload = candidate_payload(matches, candidate_id)?;
    Ok(json!({
        "candidateId":candidate_id,
        "source":exact_source_detail(sources, payload),
    }))
}

pub fn source_envelope_by_candidate(
    context: &ContextObject,
    candidate_id: &str,
) -> Result<Value, ClewError> {
    let retained = retained_context(context)?;
    let matches = retained
        .get("matches")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("retained navigation context has no match array"))?;
    let sources = retained
        .get("sources")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("retained navigation context has no source array"))?;
    let decision_payload = candidate_payload(matches, candidate_id)?;
    if decision_payload
        .get("start")
        .and_then(Value::as_u64)
        .is_none()
        || decision_payload
            .get("end")
            .and_then(Value::as_u64)
            .is_none()
        || decision_payload
            .get("startLine")
            .and_then(Value::as_u64)
            .is_none()
        || decision_payload
            .get("endLine")
            .and_then(Value::as_u64)
            .is_none()
        || decision_payload.get("rangeStart").is_some()
    {
        return singleton_source_envelope(matches, sources, candidate_id, decision_payload);
    }
    let decision_file = required_payload_string(decision_payload, "file")?;
    let decision_start_line = required_payload_u64(decision_payload, "startLine")?;
    let normalized_terms = context
        .terms
        .iter()
        .map(|term| term.to_lowercase())
        .collect::<BTreeSet<_>>();
    let mut decision = None;
    let mut exact_names = Vec::new();
    for matched in matches {
        let Some(payload) = matched.get("payload").and_then(Value::as_object) else {
            continue;
        };
        if !is_declaration(payload)
            || payload.get("file").and_then(Value::as_str) != Some(decision_file)
            || payload.get("start").and_then(Value::as_u64).is_none()
            || payload.get("rangeStart").is_some()
        {
            continue;
        }
        let compilation = required_string(matched, "compilation")?;
        let fact_key = required_string(matched, "factKey")?;
        let handle = candidate_handle(compilation, fact_key)?;
        if handle == candidate_id {
            decision = Some((matched, payload, handle));
        } else if exact_query_name_match(payload, &normalized_terms) {
            let start_line = required_payload_u64(payload, "startLine")?;
            exact_names.push((
                start_line.abs_diff(decision_start_line),
                start_line,
                fact_key,
                matched,
                payload,
                handle,
            ));
        }
    }
    let decision = decision.ok_or_else(|| {
        ClewError::new(
            ErrorCode::SymbolNotFound,
            "navigation candidate is not retained by this context",
        )
    })?;
    exact_names.sort_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then_with(|| left.1.cmp(&right.1))
            .then_with(|| left.2.cmp(right.2))
    });
    let eligible_count = 1usize.saturating_add(exact_names.len());
    let decision_window = source_window_term_hits(sources, decision.1, &normalized_terms);
    let mut selected = vec![decision];
    let mut selected_window_ranges = BTreeSet::new();
    let mut covered_terms = BTreeSet::new();
    if let Some((range, hits)) = decision_window {
        selected_window_ranges.insert(range);
        covered_terms.extend(hits);
    }
    for (_, _, _, matched, payload, handle) in exact_names {
        if selected.len() >= MAX_DECISION_SOURCE_DECLARATIONS {
            break;
        }
        let Some((range, hits)) = source_window_term_hits(sources, payload, &normalized_terms)
        else {
            continue;
        };
        let same_window = selected_window_ranges.contains(&range);
        let adds_term = hits.iter().any(|term| !covered_terms.contains(term));
        if !same_window && !adds_term && !selected_window_ranges.is_empty() {
            continue;
        }
        selected_window_ranges.insert(range);
        covered_terms.extend(hits);
        selected.push((matched, payload, handle));
    }

    let mut content_ref = None;
    let mut complete_file = false;
    let mut windows = BTreeMap::<(u64, u64, String), Value>::new();
    let mut pending_bindings = Vec::new();
    let mut source_bytes = 0usize;
    let mut truncated = eligible_count > selected.len();
    let mut source_reduced = false;
    let mut observed_exact_source = false;
    for (matched, payload, handle) in selected {
        let Some(source) = exact_source(sources, payload) else {
            truncated = true;
            continue;
        };
        observed_exact_source = true;
        let observed_content_ref = source
            .row
            .get("contentRef")
            .cloned()
            .ok_or_else(|| invalid("exact navigation source has no content authority"))?;
        if content_ref
            .as_ref()
            .is_some_and(|expected| expected != &observed_content_ref)
        {
            return Err(invalid(
                "decision source declarations have different content authority",
            ));
        }
        content_ref.get_or_insert(observed_content_ref);
        complete_file |= source
            .row
            .get("completeFile")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let mut start_line = required_value_u64(source.window, "startLine")?;
        let mut end_line = required_value_u64(source.window, "endLine")?;
        let mut text = required_value_string(source.window, "text")?.to_owned();
        let mut window_key = (start_line, end_line, text.clone());
        if !windows.contains_key(&window_key)
            && source_bytes.saturating_add(text.len()) > MAX_DECISION_SOURCE_BYTES
            && let Some(exact_text) = exact_declaration_text(sources, payload)
        {
            start_line = required_payload_u64(payload, "startLine")?;
            end_line = required_payload_u64(payload, "endLine")?;
            text = exact_text;
            window_key = (start_line, end_line, text.clone());
            truncated = true;
            source_reduced = true;
        }
        if !windows.contains_key(&window_key) {
            if source_bytes.saturating_add(text.len()) > MAX_DECISION_SOURCE_BYTES {
                truncated = true;
                source_reduced = true;
                continue;
            }
            source_bytes = source_bytes.saturating_add(text.len());
            windows.insert(
                window_key.clone(),
                json!({"startLine":start_line,"endLine":end_line,"text":text}),
            );
        }
        pending_bindings.push((
            window_key,
            json!({
                "candidateId":handle,
                "candidateKey":{
                    "compilation":required_string(matched, "compilation")?,
                    "factKey":required_string(matched, "factKey")?,
                },
                "displayName":display_name(payload),
                "declarationStartLine":payload.get("startLine"),
                "declarationEndLine":payload.get("endLine"),
            }),
        ));
    }
    let window_indexes = windows
        .keys()
        .enumerate()
        .map(|(index, key)| (key.clone(), index))
        .collect::<BTreeMap<_, _>>();
    let bindings = pending_bindings
        .into_iter()
        .map(|(window_key, mut binding)| {
            binding["windowIndex"] = json!(window_indexes[&window_key]);
            binding
        })
        .collect::<Vec<_>>();
    if bindings.is_empty() {
        return Ok(json!({
            "candidateId":candidate_id,
            "source":{
                "status":"UNSUPPORTED",
                "reason":if observed_exact_source {
                    "SOURCE_ENVELOPE_BUDGET_EXCEEDED"
                } else {
                    "NO_EXACT_DECLARATION_WINDOW"
                },
            },
        }));
    }
    let returned_binding_count = bindings.len();
    Ok(json!({
        "candidateId":candidate_id,
        "selectionAuthority":"TOP_CANDIDATE_PLUS_EXACT_QUERY_NAMES_SAME_FILE",
        "sourceBytes":source_bytes,
        "sourceBindingCount":{
            "eligible":eligible_count,
            "returned":returned_binding_count,
            "omitted":eligible_count.saturating_sub(returned_binding_count),
        },
        "sourceBindings":bindings,
        "source":{
            "status":"SUPPORTED",
            "authority":"EXACT_SNAPSHOT_TEXT",
            "fileId":decision_file,
            "contentRef":content_ref,
            "completeFile":complete_file && !source_reduced,
            "windows":windows.into_values().collect::<Vec<_>>(),
            "truncated":truncated,
        },
    }))
}

fn singleton_source_envelope(
    matches: &[Value],
    sources: &[Value],
    candidate_id: &str,
    payload: &Map<String, Value>,
) -> Result<Value, ClewError> {
    let Some(source) = exact_source(sources, payload) else {
        return Ok(json!({
            "candidateId":candidate_id,
            "source":{"status":"UNSUPPORTED","reason":"NO_EXACT_DECLARATION_WINDOW"},
        }));
    };
    let mut matched_candidate = None;
    for matched in matches {
        let compilation = required_string(matched, "compilation")?;
        let fact_key = required_string(matched, "factKey")?;
        if candidate_handle(compilation, fact_key)? == candidate_id {
            matched_candidate = Some((compilation, fact_key));
            break;
        }
    }
    let (compilation, fact_key) = matched_candidate.ok_or_else(|| {
        ClewError::new(
            ErrorCode::SymbolNotFound,
            "navigation candidate is not retained by this context",
        )
    })?;
    let declaration_start = required_payload_u64(payload, "startLine")?;
    let declaration_end = required_payload_u64(payload, "endLine")?;
    let mut start_line = required_value_u64(source.window, "startLine")?;
    let mut end_line = required_value_u64(source.window, "endLine")?;
    let mut text = required_value_string(source.window, "text")?.to_owned();
    let mut truncated = false;
    if text.len() > MAX_DECISION_SOURCE_BYTES
        && let Some(exact_text) = exact_declaration_text(sources, payload)
    {
        start_line = declaration_start;
        end_line = declaration_end;
        text = exact_text;
        truncated = true;
    }
    if text.len() > MAX_DECISION_SOURCE_BYTES {
        return Ok(json!({
            "candidateId":candidate_id,
            "source":{"status":"UNSUPPORTED","reason":"SOURCE_ENVELOPE_BUDGET_EXCEEDED"},
        }));
    }
    let content_ref = source
        .row
        .get("contentRef")
        .cloned()
        .ok_or_else(|| invalid("exact navigation source has no content authority"))?;
    let display_name =
        display_name(payload).ok_or_else(|| invalid("navigation candidate has no display name"))?;
    Ok(json!({
        "candidateId":candidate_id,
        "selectionAuthority":"TOP_CANDIDATE",
        "sourceBytes":text.len(),
        "sourceBindingCount":{"eligible":1,"returned":1,"omitted":0},
        "sourceBindings":[{
            "candidateId":candidate_id,
            "candidateKey":{"compilation":compilation,"factKey":fact_key},
            "displayName":display_name,
            "declarationStartLine":declaration_start,
            "declarationEndLine":declaration_end,
            "windowIndex":0,
        }],
        "source":{
            "status":"SUPPORTED",
            "authority":"EXACT_SNAPSHOT_TEXT",
            "fileId":required_payload_string(payload, "file")?,
            "contentRef":content_ref,
            "completeFile":source
                .row
                .get("completeFile")
                .and_then(Value::as_bool)
                .unwrap_or(false) && !truncated,
            "windows":[{"startLine":start_line,"endLine":end_line,"text":text}],
            "truncated":truncated,
        },
    }))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectReferenceSelection {
    terminal_name: String,
    source_file: String,
    matched_terms: Vec<String>,
    observations: Vec<DirectReferenceObservation>,
    source_references_truncated: bool,
}

impl DirectReferenceSelection {
    pub fn terminal_name(&self) -> &str {
        &self.terminal_name
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct DirectReferenceObservation {
    path_segments: Vec<String>,
    range_start: u64,
    range_end: u64,
    start_line: u64,
    end_line: u64,
}

struct DirectReferenceGroups {
    source_file: String,
    references: BTreeMap<String, BTreeSet<DirectReferenceObservation>>,
    truncated: bool,
}

fn direct_reference_groups(
    context: &ContextObject,
    candidate_id: &str,
) -> Result<Option<DirectReferenceGroups>, ClewError> {
    let retained = retained_context(context)?;
    let matches = retained
        .get("matches")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("retained navigation context has no match array"))?;
    let payload = candidate_payload(matches, candidate_id)?;
    direct_reference_groups_from_payload(payload)
}

fn direct_reference_groups_from_payload(
    payload: &Map<String, Value>,
) -> Result<Option<DirectReferenceGroups>, ClewError> {
    let references = match payload.get("directReferences") {
        Some(references) => references,
        None if payload.contains_key("directReferencesTruncated") => {
            return Err(invalid(
                "direct reference truncation exists without reference facts",
            ));
        }
        None => return Ok(None),
    };
    let source_file = required_payload_string(payload, "file")?.to_owned();
    let declaration_start = required_payload_u64(payload, "rangeStart")?;
    let declaration_end = required_payload_u64(payload, "rangeEnd")?;
    let declaration_start_line = required_payload_u64(payload, "startLine")?;
    let declaration_end_line = required_payload_u64(payload, "endLine")?;
    let truncated = payload
        .get("directReferencesTruncated")
        .and_then(Value::as_bool)
        .ok_or_else(|| invalid("direct reference facts have no truncation boundary"))?;
    let references = references
        .as_array()
        .ok_or_else(|| invalid("direct reference facts are not an array"))?;
    let mut grouped = BTreeMap::<String, BTreeSet<DirectReferenceObservation>>::new();
    for reference in references {
        if reference.get("kind").and_then(Value::as_str) != Some("CALL_PATH")
            || reference.get("resolution").and_then(Value::as_str) != Some("SYNTAX_UNRESOLVED")
        {
            return Err(invalid(
                "direct reference fact has an unsupported closed value",
            ));
        }
        let path_segments = reference
            .get("pathSegments")
            .and_then(Value::as_array)
            .filter(|segments| !segments.is_empty() && segments.len() <= 32)
            .ok_or_else(|| invalid("direct reference fact has invalid path segments"))?
            .iter()
            .map(|segment| {
                segment
                    .as_str()
                    .filter(|segment| !segment.is_empty() && segment.len() <= 256)
                    .map(str::to_owned)
                    .ok_or_else(|| invalid("direct reference path segment is invalid"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let terminal_name = required_value_string(reference, "terminalName")?;
        if path_segments.last().map(String::as_str) != Some(terminal_name) {
            return Err(invalid(
                "direct reference terminal does not match its final path segment",
            ));
        }
        let range_start = required_value_u64(reference, "rangeStart")?;
        let range_end = required_value_u64(reference, "rangeEnd")?;
        let start_line = required_value_u64(reference, "startLine")?;
        let end_line = required_value_u64(reference, "endLine")?;
        if range_start >= range_end
            || range_start < declaration_start
            || declaration_end < range_end
            || start_line < declaration_start_line
            || declaration_end_line < end_line
            || start_line > end_line
        {
            return Err(invalid(
                "direct reference range escapes its declaration authority",
            ));
        }
        grouped
            .entry(terminal_name.to_owned())
            .or_default()
            .insert(DirectReferenceObservation {
                path_segments,
                range_start,
                range_end,
                start_line,
                end_line,
            });
    }
    Ok(Some(DirectReferenceGroups {
        source_file,
        references: grouped,
        truncated,
    }))
}

pub fn select_direct_references(
    context: &ContextObject,
    candidate_id: &str,
) -> Result<(Vec<DirectReferenceSelection>, bool, usize), ClewError> {
    let Some(groups) = direct_reference_groups(context, candidate_id)? else {
        return Ok((Vec::new(), false, 0));
    };
    let DirectReferenceGroups {
        source_file,
        references,
        truncated,
    } = groups;
    let normalized_terms = context
        .terms
        .iter()
        .map(|term| term.to_lowercase())
        .filter(|term| !term.is_empty())
        .collect::<BTreeSet<_>>();
    let mut ranked = references
        .into_iter()
        .filter_map(|(terminal_name, observations)| {
            let components = terminal_name
                .split(|character: char| !character.is_alphanumeric())
                .filter(|component| !component.is_empty())
                .map(str::to_lowercase)
                .collect::<BTreeSet<_>>();
            let matched_terms = normalized_terms
                .intersection(&components)
                .cloned()
                .collect::<Vec<_>>();
            (!matched_terms.is_empty()).then(|| DirectReferenceSelection {
                terminal_name,
                source_file: source_file.clone(),
                matched_terms,
                observations: observations.into_iter().collect(),
                source_references_truncated: truncated,
            })
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|left, right| {
        right
            .matched_terms
            .len()
            .cmp(&left.matched_terms.len())
            .then_with(|| right.observations.len().cmp(&left.observations.len()))
            .then_with(|| left.terminal_name.cmp(&right.terminal_name))
    });
    let eligible_count = ranked.len();
    ranked.truncate(MAX_REFERENCE_FOLLOW_TERMS);
    Ok((ranked, truncated, eligible_count))
}

pub fn select_explicit_direct_references(
    context: &ContextObject,
    candidate_id: &str,
    requested: &[String],
) -> Result<Vec<DirectReferenceSelection>, ClewError> {
    if requested.is_empty()
        || requested.len() > MAX_REFERENCE_FOLLOW_TERMS
        || requested.iter().collect::<BTreeSet<_>>().len() != requested.len()
    {
        return Err(invalid(
            "explicit reference selection requires one to three unique references",
        ));
    }
    let retained = retained_context(context)?;
    let matches = retained
        .get("matches")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("retained navigation context has no match array"))?;
    let payload = candidate_payload(matches, candidate_id)?;
    if !supports_explicit_direct_references(payload) {
        return Err(ClewError::new(
            ErrorCode::UnsupportedProjectConfiguration,
            "explicit direct-reference follow is unsupported for this fact schema",
        ));
    }
    let groups = direct_reference_groups_from_payload(payload)?;
    let truncated = groups.as_ref().is_some_and(|groups| groups.truncated);
    let Some(groups) = groups else {
        return Err(ClewError::new(
            ErrorCode::SymbolNotFound,
            "selected candidate has no retained direct references",
        ));
    };
    let mut terminal_names = BTreeSet::new();
    let mut selections = Vec::new();
    for requested in requested {
        let path = requested.split("::").collect::<Vec<_>>();
        if requested.is_empty()
            || requested.len() > MAX_REFERENCE_PATH_BYTES
            || path.iter().any(|segment| {
                segment.is_empty() || segment.len() > 256 || segment.chars().any(char::is_control)
            })
            || path.len() > 32
        {
            return Err(invalid("explicit reference selector is invalid"));
        }
        let terminal_name = path.last().expect("nonempty path").to_string();
        if !terminal_names.insert(terminal_name.clone()) {
            return Err(invalid(
                "explicit reference selectors have duplicate terminal names",
            ));
        }
        let observations = groups
            .references
            .get(&terminal_name)
            .into_iter()
            .flatten()
            .filter(|observation| {
                path.len() == 1
                    || observation
                        .path_segments
                        .iter()
                        .map(String::as_str)
                        .eq(path.iter().copied())
            })
            .cloned()
            .collect::<Vec<_>>();
        if observations.is_empty() {
            return Err(ClewError::new(
                if truncated {
                    ErrorCode::IncompleteSemanticAnalysis
                } else {
                    ErrorCode::SymbolNotFound
                },
                format!("direct reference {requested} is not retained by the selected candidate"),
            ));
        }
        selections.push(DirectReferenceSelection {
            terminal_name,
            source_file: groups.source_file.clone(),
            matched_terms: vec![requested.clone()],
            observations,
            source_references_truncated: groups.truncated,
        });
    }
    Ok(selections)
}

pub fn direct_reference_detail(
    context: &ContextObject,
    selections: &[DirectReferenceSelection],
    eligible_count: usize,
) -> Result<Value, ClewError> {
    direct_reference_detail_with_authority(
        context,
        selections,
        eligible_count,
        "LEXICAL_QUERY_TERM_OVERLAP",
    )
}

pub fn explicit_direct_reference_detail(
    context: &ContextObject,
    selections: &[DirectReferenceSelection],
) -> Result<Value, ClewError> {
    direct_reference_detail_with_authority(
        context,
        selections,
        selections.len(),
        "USER_SELECTED_RETAINED_REFERENCE",
    )
}

fn direct_reference_detail_with_authority(
    context: &ContextObject,
    selections: &[DirectReferenceSelection],
    eligible_count: usize,
    selection_authority: &str,
) -> Result<Value, ClewError> {
    if selections.is_empty()
        || selections.len() > MAX_REFERENCE_FOLLOW_TERMS
        || eligible_count < selections.len()
    {
        return Err(invalid(
            "direct reference detail requires a nonempty bounded selection",
        ));
    }
    let retained = retained_context(context)?;
    let matches = retained
        .get("matches")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("retained navigation context has no match array"))?;
    let sources = retained
        .get("sources")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("retained navigation context has no source array"))?;
    let mut source_bytes = 0usize;
    let source_references_truncated = selections
        .iter()
        .any(|selection| selection.source_references_truncated);
    let selection_truncated = eligible_count > selections.len();
    let mut follow_truncated =
        source_references_truncated || selection_truncated || retained_query_truncated(context);
    let mut details = Vec::new();
    for selection in selections {
        let mut candidates = matches
            .iter()
            .filter_map(|matched| {
                let payload = matched.get("payload").and_then(Value::as_object)?;
                (is_declaration(payload)
                    && exact_candidate_name_matches(payload, &selection.terminal_name))
                .then_some((matched, payload))
            })
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| {
            let left_file = left.1.get("file").and_then(Value::as_str).unwrap_or("");
            let right_file = right.1.get("file").and_then(Value::as_str).unwrap_or("");
            let left_same_file = left_file == selection.source_file;
            let right_same_file = right_file == selection.source_file;
            right_same_file
                .cmp(&left_same_file)
                .then_with(|| left_file.cmp(right_file))
                .then_with(|| {
                    left.1
                        .get("rangeStart")
                        .and_then(Value::as_u64)
                        .cmp(&right.1.get("rangeStart").and_then(Value::as_u64))
                })
                .then_with(|| {
                    left.0
                        .get("factKey")
                        .and_then(Value::as_str)
                        .cmp(&right.0.get("factKey").and_then(Value::as_str))
                })
        });
        let total = candidates.len();
        let mut returned = Vec::new();
        for (matched, payload) in candidates.into_iter().take(MAX_REFERENCE_MATCHES_PER_TERM) {
            let mut source = exact_source_detail(sources, payload);
            let exact_bytes = source
                .pointer("/windows/0/text")
                .and_then(Value::as_str)
                .map(str::len)
                .unwrap_or(0);
            if source_bytes.saturating_add(exact_bytes) > MAX_REFERENCE_FOLLOW_SOURCE_BYTES {
                source = json!({"status":"UNAVAILABLE","reason":"OMITTED_BUDGET"});
                follow_truncated = true;
            } else {
                source_bytes = source_bytes.saturating_add(exact_bytes);
            }
            returned.push(json!({
                "candidateId":candidate_handle(
                    required_string(matched, "compilation")?,
                    required_string(matched, "factKey")?,
                )?,
                "candidateKey":{
                    "compilation":required_string(matched, "compilation")?,
                    "factKey":required_string(matched, "factKey")?,
                },
                "displayName":display_name(payload),
                "location":{
                    "file":payload.get("file"),
                    "startLine":payload.get("startLine"),
                    "endLine":payload.get("endLine"),
                    "start":payload.get("rangeStart").or_else(|| payload.get("start")),
                    "end":payload.get("rangeEnd").or_else(|| payload.get("end")),
                },
                "sameFileAsObservation":payload.get("file").and_then(Value::as_str) == Some(selection.source_file.as_str()),
                "source":source,
            }));
        }
        let observations = selection
            .observations
            .iter()
            .filter(|observation| {
                observation
                    .path_segments
                    .iter()
                    .map(String::len)
                    .sum::<usize>()
                    .saturating_add(observation.path_segments.len().saturating_sub(1) * 2)
                    <= MAX_REFERENCE_PATH_BYTES
            })
            .take(MAX_REFERENCE_OBSERVATIONS_PER_TERM)
            .map(|observation| {
                json!({
                    "pathSegments":observation.path_segments,
                    "rangeStart":observation.range_start,
                    "rangeEnd":observation.range_end,
                    "startLine":observation.start_line,
                    "endLine":observation.end_line,
                })
            })
            .collect::<Vec<_>>();
        let observation_total = selection.observations.len();
        follow_truncated |= observations.len() < observation_total;
        follow_truncated |= returned.len() < total;
        details.push(json!({
            "observed":{
                "kind":"CALL_PATH",
                "sourceFile":selection.source_file,
                "terminalName":selection.terminal_name,
                "matchedTerms":selection.matched_terms,
                "sourceReferencesTruncated":selection.source_references_truncated,
                "occurrenceCount":{
                    "returned":observations.len(),
                    "total":observation_total,
                    "omitted":observation_total.saturating_sub(observations.len()),
                },
                "occurrences":observations,
            },
            "selectionAuthority":selection_authority,
            "targetResolution":"UNRESOLVED",
            "semanticRelation":"UNKNOWN",
            "nameMatches":{
                "status":match total {
                    0 => "NOT_FOUND_IN_RETAINED_CONTEXT",
                    1 => "UNIQUE_RETAINED_NAME",
                    _ => "AMBIGUOUS_RETAINED_NAME",
                },
                "ordering":"SAME_FILE_FIRST_PRESENTATION_ONLY",
                "returned":returned.len(),
                "total":total,
                "omitted":total.saturating_sub(returned.len()),
                "candidates":returned,
            },
        }));
    }
    Ok(json!({
        "schema":"codeclew-navigation-reference-follow/1.0",
        "status":"SUPPORTED",
        "sessionId":context.session_id,
        "contextId":context.context_id,
        "evidenceDigest":context.evidence_digest,
        "selectionAuthority":selection_authority,
        "targetResolution":"UNRESOLVED",
        "semanticRelation":"UNKNOWN",
        "sourceBytes":source_bytes,
        "sourceReferencesTruncated":source_references_truncated,
        "referenceTermCount":{
            "returned":selections.len(),
            "eligible":eligible_count,
            "omitted":eligible_count.saturating_sub(selections.len()),
        },
        "selectionTruncated":selection_truncated,
        "references":details,
        "contextCompleteness":retained.get("completeness"),
        "truncated":follow_truncated,
    }))
}

fn candidate_payload<'a>(
    matches: &'a [Value],
    candidate_id: &str,
) -> Result<&'a Map<String, Value>, ClewError> {
    let mut selected = None;
    for matched in matches {
        let Some(payload) = matched.get("payload").and_then(Value::as_object) else {
            continue;
        };
        if !is_declaration(payload) {
            continue;
        }
        if candidate_handle(
            required_string(matched, "compilation")?,
            required_string(matched, "factKey")?,
        )? == candidate_id
            && selected.replace(payload).is_some()
        {
            return Err(invalid("navigation candidate handle is ambiguous"));
        }
    }
    let payload = selected.ok_or_else(|| {
        ClewError::new(
            ErrorCode::SymbolNotFound,
            "navigation candidate is not retained by this context",
        )
    })?;
    Ok(payload)
}

fn selected_candidate_detail(
    matches: &[Value],
    sources: &[Value],
    candidate_id: &str,
    include_source: bool,
    requested_facets: &BTreeSet<NavigationFacet>,
) -> Result<Value, ClewError> {
    let mut selected = None;
    for matched in matches {
        let Some(payload) = matched.get("payload").and_then(Value::as_object) else {
            continue;
        };
        if !is_declaration(payload) {
            continue;
        }
        let compilation = required_string(matched, "compilation")?;
        let fact_key = required_string(matched, "factKey")?;
        if candidate_handle(compilation, fact_key)? == candidate_id
            && selected.replace((matched, payload)).is_some()
        {
            return Err(invalid("navigation candidate handle is ambiguous"));
        }
    }
    let (matched, payload) = selected.ok_or_else(|| {
        ClewError::new(
            ErrorCode::SymbolNotFound,
            "navigation candidate is not retained by this context",
        )
    })?;
    let compilation = required_string(matched, "compilation")?;
    let fact_key = required_string(matched, "factKey")?;
    let identity = payload.get("symbolIdentity").and_then(Value::as_str);
    let identities = identity.into_iter().collect::<BTreeSet<_>>();
    let source = if include_source {
        exact_source_detail(sources, payload)
    } else {
        json!({"status":"NOT_REQUESTED"})
    };
    let source_delivery = source_delivery(&source)?;
    let reference_choices = bounded_reference_choices(payload, candidate_id)?;
    Ok(json!({
        "candidateId":candidate_id,
        "candidateKey":{"compilation":compilation,"factKey":fact_key},
        "displayName":display_name(payload).unwrap_or(fact_key),
        "fact":{
            "authority":"RETAINED_CAS_FACT",
            "domainUri":matched.get("domainUri"),
            "payloadRef":matched.get("payloadRef"),
            "payload":payload,
        },
        "source":source,
        "sourceDelivery":source_delivery,
        "referenceChoices":reference_choices,
        "facets":{
            "callers":relation_facet(matches, &identities, NavigationFacet::Callers, requested_facets.contains(&NavigationFacet::Callers)),
            "callees":relation_facet(matches, &identities, NavigationFacet::Callees, requested_facets.contains(&NavigationFacet::Callees)),
            "tests":relation_facet(matches, &identities, NavigationFacet::Tests, requested_facets.contains(&NavigationFacet::Tests)),
        },
    }))
}

fn bounded_reference_choices(
    payload: &Map<String, Value>,
    candidate_id: &str,
) -> Result<Value, ClewError> {
    if !supports_explicit_direct_references(payload) {
        return Ok(json!({
            "status":"UNSUPPORTED",
            "reason":"FACT_SCHEMA_HAS_NO_EXPLICIT_DIRECT_REFERENCE_CONTRACT",
            "choices":[],
        }));
    }
    let Some(groups) = direct_reference_groups_from_payload(payload)? else {
        return Ok(json!({
            "status":"UNSUPPORTED",
            "reason":"NO_RETAINED_DIRECT_REFERENCE_FACTS",
            "choices":[],
        }));
    };
    let total = groups.references.len();
    let mut truncated = groups.truncated || total > MAX_REFERENCE_FOLLOW_TERMS;
    let mut choices = Vec::new();
    let mut ranked = groups.references.iter().collect::<Vec<_>>();
    ranked.sort_by(|left, right| {
        let left_qualified = left
            .1
            .iter()
            .any(|observation| observation.path_segments.len() > 1);
        let right_qualified = right
            .1
            .iter()
            .any(|observation| observation.path_segments.len() > 1);
        right_qualified
            .cmp(&left_qualified)
            .then_with(|| left.0.cmp(right.0))
    });
    for (terminal_name, observations) in ranked.into_iter().take(MAX_REFERENCE_FOLLOW_TERMS) {
        let mut ranked_paths = observations
            .iter()
            .filter(|observation| {
                observation
                    .path_segments
                    .iter()
                    .map(String::len)
                    .sum::<usize>()
                    .saturating_add(observation.path_segments.len().saturating_sub(1) * 2)
                    <= MAX_REFERENCE_PATH_BYTES
            })
            .collect::<Vec<_>>();
        ranked_paths.sort_by(|left, right| {
            let left_qualified = left.path_segments.len() > 1;
            let right_qualified = right.path_segments.len() > 1;
            right_qualified
                .cmp(&left_qualified)
                .then_with(|| left.cmp(right))
        });
        let paths = ranked_paths
            .into_iter()
            .take(MAX_REFERENCE_OBSERVATIONS_PER_TERM)
            .map(|observation| observation.path_segments.join("::"))
            .collect::<Vec<_>>();
        let returned = paths.len();
        truncated |= returned < observations.len();
        choices.push(json!({
            "terminalName":terminal_name,
            "paths":paths,
            "observationCount":{
                "returned":returned,
                "total":observations.len(),
                "omitted":observations.len().saturating_sub(returned),
            },
        }));
    }
    Ok(json!({
        "status":"SUPPORTED",
        "selectionAuthority":"RETAINED_DIRECT_REFERENCE_FACT",
        "returned":choices.len(),
        "total":total,
        "truncated":truncated,
        "choices":choices,
        "nextAction":format!(
            "nav expand --session <sessionId> --from <contextId> --candidate {candidate_id} --reference <terminal-or-full-path> --source"
        ),
        "followAction":{
            "kind":"RETAINED_REFERENCE_FOLLOW",
            "candidateId":candidate_id,
            "maxReferences":MAX_REFERENCE_FOLLOW_TERMS,
            "onePathPerTerminal":true,
            "includeSource":true,
            "includeFacet":false,
            "requiresNewestContext":true,
            "choiceAuthority":"RETAINED_DIRECT_REFERENCE_FACT",
            "resultSelectionAuthority":"USER_SELECTED_RETAINED_REFERENCE",
            "targetResolution":"UNRESOLVED",
            "semanticRelation":"UNKNOWN",
        },
    }))
}

fn source_delivery(source: &Value) -> Result<Value, ClewError> {
    match source.get("status").and_then(Value::as_str) {
        Some("SUPPORTED") => Ok(json!({
            "status":"RETURNED",
            "reuse":"CURRENT_RESULT",
            "repeatSameRequest":false,
        })),
        Some("NOT_REQUESTED") => Ok(json!({
            "status":"NOT_RETURNED",
            "requestAction":"candidateSource",
        })),
        Some("UNSUPPORTED") => Ok(json!({
            "status":"UNAVAILABLE",
            "reason":required_value_string(source, "reason")?,
            "repeatSameRequest":false,
        })),
        _ => Err(invalid("navigation candidate source has an invalid status")),
    }
}

fn supports_explicit_direct_references(payload: &Map<String, Value>) -> bool {
    payload.get("schema").and_then(Value::as_str) == Some("codeclew-rust-syntax-fact/1.2")
}

pub fn validate_stdout(value: &Value) -> Result<(), ClewError> {
    let bytes = canonical::bytes(value).map_err(internal)?;
    if bytes.len().saturating_add(1) > MAX_NAV_STDOUT_BYTES {
        return Err(ClewError::new(
            ErrorCode::SliceBudgetExceeded,
            "navigation stdout exceeds 64 KiB; narrow terms or roots",
        ));
    }
    Ok(())
}

// The public result is bound independently to session, context, evidence,
// request, retained facts, truncation, and facets; merging those authorities
// into a convenience object would make this validation boundary less explicit.
#[allow(clippy::too_many_arguments)]
fn assemble(
    session_id: &str,
    context_id: &str,
    evidence_digest: &str,
    intent: &str,
    terms: &[String],
    retained: &Value,
    query_truncated: bool,
    requested_facets: &[NavigationFacet],
) -> Result<Value, ClewError> {
    assemble_with_decision_identifier(
        session_id,
        context_id,
        evidence_digest,
        intent,
        terms,
        retained,
        query_truncated,
        requested_facets,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
fn assemble_with_decision_identifier(
    session_id: &str,
    context_id: &str,
    evidence_digest: &str,
    intent: &str,
    terms: &[String],
    retained: &Value,
    query_truncated: bool,
    requested_facets: &[NavigationFacet],
    decision_identifier: Option<&str>,
) -> Result<Value, ClewError> {
    let matches = retained
        .get("matches")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("retained navigation context has no match array"))?;
    let sources = retained
        .get("sources")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("retained navigation context has no source array"))?;

    let normalized_terms = crate::query_v2::normalize_terms(terms.iter().map(String::as_str))
        .into_iter()
        .collect::<BTreeSet<_>>();
    if let Some(identifier) = decision_identifier {
        if identifier.is_empty()
            || identifier.len() > 1024
            || identifier.trim() != identifier
            || identifier.chars().any(char::is_control)
        {
            return Err(invalid("navigation decision identifier is invalid"));
        }
        let identifier_terms = crate::query_v2::normalize_terms(std::iter::once(identifier));
        if identifier_terms.is_empty()
            || identifier_terms
                .iter()
                .any(|term| !normalized_terms.contains(term))
        {
            return Err(invalid(
                "navigation decision identifier is not bound to the query terms",
            ));
        }
    }
    let mut ranked_candidates = Vec::new();
    let mut handles = BTreeMap::new();
    for (ordinal, matched) in matches.iter().enumerate() {
        let Some(payload) = matched.get("payload").and_then(Value::as_object) else {
            continue;
        };
        if !is_declaration(payload) {
            continue;
        }
        let fact_key = required_string(matched, "factKey")?;
        let compilation = required_string(matched, "compilation")?;
        let candidate_id = candidate_handle(compilation, fact_key)?;
        if handles
            .insert(candidate_id.clone(), (compilation, fact_key))
            .is_some()
        {
            return Err(invalid("navigation candidate handle collision"));
        }
        let covered_terms = candidate_authority_term_hits(payload, sources, &normalized_terms);
        let decision_identifier_match = decision_identifier
            .is_some_and(|identifier| candidate_matches_declared_identifier(payload, identifier));
        let (term_coverage, name_coverage, occurrences) =
            candidate_relevance(payload, sources, &normalized_terms);
        let (window_coverage, window_occurrences) =
            source_window_relevance(sources, payload, &normalized_terms);
        ranked_candidates.push((
            usize::from(decision_identifier_match),
            window_coverage,
            term_coverage,
            name_coverage,
            occurrences,
            window_occurrences,
            ordinal,
            candidate_id,
            matched,
            payload,
            covered_terms,
            decision_identifier_match,
        ));
    }
    ranked_candidates.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| right.1.cmp(&left.1))
            .then_with(|| right.2.cmp(&left.2))
            .then_with(|| right.3.cmp(&left.3))
            .then_with(|| right.4.cmp(&left.4))
            .then_with(|| right.5.cmp(&left.5))
            .then_with(|| left.6.cmp(&right.6))
    });
    let total_candidates = ranked_candidates.len();
    let full_coverage_candidate_count = (!normalized_terms.is_empty())
        .then(|| {
            ranked_candidates
                .iter()
                .filter(|candidate| candidate.10.len() == normalized_terms.len())
                .count()
        })
        .unwrap_or(0);
    let exact_identifier_candidate_count = ranked_candidates
        .iter()
        .filter(|candidate| candidate.0 > 0)
        .count();
    let mut candidates = Vec::new();
    let mut returned_candidate_coverage = BTreeMap::new();
    let mut returned_candidate_exact_identifiers = BTreeMap::new();
    let mut candidate_identities = BTreeSet::new();
    for (
        _,
        _,
        _,
        _,
        _,
        _,
        _,
        candidate_id,
        matched,
        payload,
        covered_terms,
        decision_identifier_match,
    ) in ranked_candidates.into_iter().take(MAX_NAV_CANDIDATES)
    {
        let fact_key = required_string(matched, "factKey")?;
        let compilation = required_string(matched, "compilation")?;
        if let Some(identity) = payload.get("symbolIdentity").and_then(Value::as_str) {
            candidate_identities.insert(identity);
        }
        let file = payload.get("file").and_then(Value::as_str);
        let (display_name, display_name_truncated) =
            bounded_line_prefix(display_name(payload).unwrap_or(fact_key), 512);
        let (declaration_kind, declaration_kind_truncated) =
            bounded_optional_string(payload.get("declarationKind").and_then(Value::as_str), 512);
        let (symbol_identity, symbol_identity_truncated) =
            bounded_optional_string(payload.get("symbolIdentity").and_then(Value::as_str), 512);
        let (file, file_truncated) = bounded_optional_string(file, 512);
        let preview = exact_source_preview(sources, payload);
        returned_candidate_coverage.insert(candidate_id.clone(), covered_terms);
        returned_candidate_exact_identifiers
            .insert(candidate_id.clone(), decision_identifier_match);
        candidates.push(json!({
            "candidateId":candidate_id,
            "candidateKey":{"compilation":compilation,"factKey":fact_key},
            "displayName":display_name,
            "displayNameTruncated":display_name_truncated,
            "declarationKind":declaration_kind,
            "declarationKindTruncated":declaration_kind_truncated,
            "symbolIdentity":symbol_identity,
            "symbolIdentityTruncated":symbol_identity_truncated,
            "location":{
                "file":file,
                "fileTruncated":file_truncated,
                "startLine":payload.get("startLine"),
                "endLine":payload.get("endLine"),
                "start":payload.get("start").or_else(|| payload.get("rangeStart")),
                "end":payload.get("end").or_else(|| payload.get("rangeEnd")),
            },
            "preview":preview,
        }));
    }

    let requested_facets = requested_facets.iter().copied().collect::<BTreeSet<_>>();
    let callers = relation_facet(
        matches,
        &candidate_identities,
        NavigationFacet::Callers,
        requested_facets.contains(&NavigationFacet::Callers),
    );
    let callees = relation_facet(
        matches,
        &candidate_identities,
        NavigationFacet::Callees,
        requested_facets.contains(&NavigationFacet::Callees),
    );
    let tests = relation_facet(
        matches,
        &candidate_identities,
        NavigationFacet::Tests,
        requested_facets.contains(&NavigationFacet::Tests),
    );

    let decision_authority = navigation_decision_authority(
        session_id,
        context_id,
        &normalized_terms,
        &candidates,
        &returned_candidate_coverage,
        &returned_candidate_exact_identifiers,
        full_coverage_candidate_count,
        exact_identifier_candidate_count,
        decision_identifier.is_some(),
        query_truncated,
        total_candidates > candidates.len(),
        retained.get("completeness"),
    )?;
    let result = json!({
        "schema":NAV_RESULT_SCHEMA,
        "sessionId":session_id,
        "contextId":context_id,
        "evidenceDigest":evidence_digest,
        "intent":intent,
        "terms":terms,
        "candidates":candidates,
        "termAnchors":term_anchors(sources, terms),
        "candidateCount":{
            "returned":candidates.len(),
            "total":total_candidates,
            "omitted":total_candidates.saturating_sub(candidates.len()),
        },
        "queryCoverageTruncated":query_truncated,
        "candidateListTruncated":total_candidates > candidates.len(),
        "decisionAuthority":decision_authority,
        "facets":{
            "declaration":supported("RETAINED_DECLARATION_FACT"),
            "source":supported("BOUNDED_SESSION_SOURCE"),
            "callers":callers,
            "callees":callees,
            "tests":tests,
        },
        "completeness":retained.get("completeness"),
        "truncated":query_truncated || total_candidates > candidates.len(),
        "nextAction":{
            "detail":"nav expand --session <sessionId> --from <contextId> --candidate <candidateId> [--candidate <candidateId> ...] [--source] [--facet callers|callees|tests]",
            "refine":"nav expand --session <sessionId> --from <contextId> --term <additional-term>",
            "exactSource":"nav expand --session <sessionId> --from <contextId> --term <exact-identifier> --file <repository-relative-file> --source",
        },
        "nextActions":{
            "schema":NAV_ACTION_SCHEMA,
            "candidateSource":{
                "kind":"CANDIDATE_SOURCE",
                "maxCandidates":MAX_NAV_CANDIDATES,
                "includeSource":true,
                "includeFacet":false,
            },
            "exactSource":{
                "kind":"EXACT_FILE_SOURCE",
                "maxTerms":MAX_NAV_CANDIDATES,
                "sameFileRequired":true,
                "includeSource":true,
            },
            "facet":{
                "kind":"CANDIDATE_FACET",
                "allowedValues":["callers","callees","tests"],
                "includeSource":false,
                "explicitRelationOnly":true,
            },
            "refine":{
                "kind":"TERM_REFINEMENT",
                "includeSource":false,
            },
        },
    });
    Ok(result)
}

fn navigation_decision_authority(
    session_id: &str,
    context_id: &str,
    required_terms: &BTreeSet<String>,
    candidates: &[Value],
    returned_candidate_coverage: &BTreeMap<String, BTreeSet<String>>,
    returned_candidate_exact_identifiers: &BTreeMap<String, bool>,
    full_coverage_candidate_count: usize,
    exact_identifier_candidate_count: usize,
    decision_identifier_declared: bool,
    query_truncated: bool,
    candidate_list_truncated: bool,
    completeness: Option<&Value>,
) -> Result<Value, ClewError> {
    let unmatched_terms = completeness
        .and_then(|value| value.get("unmatchedTerms"))
        .and_then(Value::as_array)
        .map(|terms| {
            terms
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_lowercase)
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_default();
    let returned_coverage = candidates
        .iter()
        .map(|candidate| {
            let candidate_id = required_value_string(candidate, "candidateId")?;
            let covered_terms = returned_candidate_coverage
                .get(candidate_id)
                .ok_or_else(|| internal("returned navigation candidate has no coverage"))?;
            let decision_identifier_match = returned_candidate_exact_identifiers
                .get(candidate_id)
                .ok_or_else(|| {
                    internal("returned navigation candidate has no exact identifier coverage")
                })?;
            let missing_terms = required_terms
                .difference(covered_terms)
                .cloned()
                .collect::<Vec<_>>();
            Ok(json!({
                "candidateId":candidate_id,
                "coveredTermCount":covered_terms.len(),
                "missingTermCount":missing_terms.len(),
                "decisionIdentifierMatch":decision_identifier_match,
                "complete":missing_terms.is_empty() && !required_terms.is_empty(),
            }))
        })
        .collect::<Result<Vec<_>, ClewError>>()?;
    let exact_identifier_candidate = returned_coverage.iter().find(|candidate| {
        candidate
            .get("decisionIdentifierMatch")
            .and_then(Value::as_bool)
            == Some(true)
    });
    if !required_terms.is_empty()
        && unmatched_terms.is_empty()
        && !query_truncated
        && exact_identifier_candidate_count == 1
        && exact_identifier_candidate
            .and_then(|candidate| candidate.get("complete"))
            .and_then(Value::as_bool)
            == Some(true)
    {
        let candidate = exact_identifier_candidate.ok_or_else(|| {
            internal("unique exact-identifier navigation candidate was not returned")
        })?;
        return Ok(json!({
            "schema":NAV_DECISION_AUTHORITY_SCHEMA,
            "status":"SUPPORTED",
            "classification":"UNIQUE_EXACT_IDENTIFIER_FULL_COVERAGE",
            "basis":"EXACT_DECLARATION_NAME_AND_BOUNDARY_SAFE_LEXICAL_COVERAGE",
            "candidateId":required_value_string(candidate, "candidateId")?,
            "requiredTermCount":required_terms.len(),
            "coveredTermCount":required_terms.len(),
            "returnedCandidateCoverage":returned_coverage,
            "observedFullCoverageCandidateCount":full_coverage_candidate_count,
            "observedExactIdentifierCandidateCount":exact_identifier_candidate_count,
            "queryCoverageTruncated":query_truncated,
            "candidateListTruncated":candidate_list_truncated,
        }));
    }

    let best_candidate_id = candidates
        .first()
        .and_then(|candidate| candidate.get("candidateId"))
        .and_then(Value::as_str);
    let best_covered_terms = best_candidate_id
        .and_then(|candidate_id| returned_candidate_coverage.get(candidate_id))
        .cloned()
        .unwrap_or_default();
    let best_missing_terms = required_terms
        .difference(&best_covered_terms)
        .cloned()
        .collect::<BTreeSet<_>>();
    let (classification, next_missing_term) = if required_terms.is_empty() {
        ("NO_EXPLICIT_TERMS", None)
    } else if !unmatched_terms.is_empty() {
        (
            "UNMATCHED_EXPLICIT_TERMS",
            unmatched_terms.iter().next().cloned(),
        )
    } else if query_truncated {
        (
            "TRUNCATED_QUERY_COVERAGE",
            best_missing_terms.iter().next().cloned(),
        )
    } else if !decision_identifier_declared {
        ("NO_DECLARED_DECISION_IDENTIFIER", None)
    } else if exact_identifier_candidate_count > 1 {
        ("AMBIGUOUS_EXACT_IDENTIFIER", None)
    } else if exact_identifier_candidate_count == 0 {
        ("NO_EXACT_IDENTIFIER", None)
    } else if exact_identifier_candidate.is_none() {
        ("EXACT_IDENTIFIER_NOT_RETURNED", None)
    } else {
        (
            "PARTIAL_EXACT_IDENTIFIER_COVERAGE",
            best_missing_terms.iter().next().cloned(),
        )
    };
    let refinement = if decision_identifier_declared {
        json!({
            "kind":"STOP_UNRESOLVED",
            "reason":"DECLARED_IDENTIFIER_DID_NOT_ESTABLISH_UNIQUE_FULL_COVERAGE",
            "repeatSameRequest":false,
            "instruction":"The declared identifier did not establish one complete decision. Do not repeat it or select an analogue. Continue only after the task or an exact retained source supplies a different identifier or a known file; otherwise keep the task unresolved.",
        })
    } else {
        json!({
            "kind":"ADD_TASK_DISCRIMINANT",
            "command":format!(
                "nav expand --session {session_id} --from {context_id} --term <new-task-derived-exact-identifier> --decision-identifier <same-new-task-derived-exact-identifier>"
            ),
            "precondition":"NEW_TASK_SUPPLIED_EXACT_IDENTIFIER_NOT_ALREADY_IN_CONTEXT",
            "repeatSameRequest":false,
            "onUnsatisfied":"STOP_UNRESOLVED",
            "requiredCoverage":next_missing_term.iter().collect::<Vec<_>>(),
            "instruction":"Use this refinement once only when the task supplies a new exact identifier that is absent from the current query. Never copy a returned card name into this field. If no such identifier is available, keep the task unresolved.",
        })
    };
    let authority = json!({
        "schema":NAV_DECISION_AUTHORITY_SCHEMA,
        "status":"ABSTAIN",
        "classification":classification,
        "basis":"EXACT_DECLARATION_NAME_AND_BOUNDARY_SAFE_LEXICAL_COVERAGE",
        "requiredTermCount":required_terms.len(),
        "unmatchedTermCount":unmatched_terms.len(),
        "bestCandidateCoveredTermCount":best_covered_terms.len(),
        "bestCandidateMissingTermCount":best_missing_terms.len(),
        "returnedCandidateCoverage":returned_coverage,
        "observedFullCoverageCandidateCount":full_coverage_candidate_count,
        "observedExactIdentifierCandidateCount":exact_identifier_candidate_count,
        "queryCoverageTruncated":query_truncated,
        "candidateListTruncated":candidate_list_truncated,
        "refinement":refinement,
    });
    Ok(authority)
}

type CandidateKey = (String, String);

fn candidate_map(value: &Value) -> Result<BTreeMap<CandidateKey, &Value>, ClewError> {
    value
        .get("candidates")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("navigation result has no candidate array"))?
        .iter()
        .map(|candidate| Ok((candidate_key(candidate)?, candidate)))
        .collect()
}

fn candidate_key(candidate: &Value) -> Result<CandidateKey, ClewError> {
    let key = candidate
        .get("candidateKey")
        .ok_or_else(|| invalid("navigation candidate has no stable key"))?;
    Ok((
        required_string(key, "compilation")?.to_owned(),
        required_string(key, "factKey")?.to_owned(),
    ))
}

fn candidate_upsert(candidate: &Value, change: &str) -> Result<Value, ClewError> {
    let mut upsert = candidate.clone();
    upsert
        .as_object_mut()
        .ok_or_else(|| invalid("navigation candidate is not an object"))?
        .insert("change".into(), Value::String(change.into()));
    Ok(upsert)
}

fn relation_facet(
    matches: &[Value],
    candidate_identities: &BTreeSet<&str>,
    facet: NavigationFacet,
    requested: bool,
) -> Value {
    if !requested {
        return json!({"status":"NOT_REQUESTED"});
    }
    if candidate_identities.is_empty() {
        return unsupported("NO_FACT_BOUND_CANDIDATE_IDENTITY");
    }
    let edges = matches
        .iter()
        .filter_map(|matched| {
            let payload = matched.get("payload")?.as_object()?;
            let kind = payload.get("kind")?.as_str()?;
            if !kind.eq_ignore_ascii_case("relation") {
                return None;
            }
            let source = payload.get("sourceIdentity")?.as_str()?;
            let target = payload.get("targetIdentity")?.as_str()?;
            let compilation = matched.get("compilation")?.as_str()?;
            let fact_key = matched.get("factKey")?.as_str()?;
            let relation_kind = payload
                .get("relationKind")
                .and_then(Value::as_str)
                .unwrap_or("");
            let selected = match facet {
                NavigationFacet::Callers => candidate_identities.contains(target),
                NavigationFacet::Callees => candidate_identities.contains(source),
                NavigationFacet::Tests => {
                    relation_kind.to_ascii_uppercase().contains("TEST")
                        && (candidate_identities.contains(source)
                            || candidate_identities.contains(target))
                }
            };
            selected.then(|| {
                json!({
                    "edgeKey":{"compilation":compilation,"factKey":fact_key},
                    "compilation":compilation,
                    "factKey":fact_key,
                    "domainUri":matched.get("domainUri"),
                    "payloadRef":matched.get("payloadRef"),
                    "relation":payload,
                })
            })
        })
        .collect::<Vec<_>>();
    if edges.is_empty() {
        unsupported(match facet {
            NavigationFacet::Callers | NavigationFacet::Callees => {
                "NO_DIRECT_RESOLVED_RELATION_FACTS_IN_BOUNDED_CONTEXT"
            }
            NavigationFacet::Tests => "NO_DIRECT_TEST_RELATION_FACTS_IN_BOUNDED_CONTEXT",
        })
    } else {
        json!({
            "status":"PARTIAL",
            "authority":"DIRECT_RESOLVED_RELATION_FACT",
            "reason":"BOUNDED_CONTEXT_RELATIONS_ONLY",
            "edges":edges,
        })
    }
}

fn is_declaration(payload: &Map<String, Value>) -> bool {
    payload
        .get("kind")
        .and_then(Value::as_str)
        .is_some_and(|kind| kind.eq_ignore_ascii_case("declaration"))
        || (payload.contains_key("declarationKind") && payload.contains_key("symbolIdentity"))
}

fn display_name(payload: &Map<String, Value>) -> Option<&str> {
    ["name", "qualifiedName", "symbolIdentity", "ownerIdentity"]
        .into_iter()
        .find_map(|key| payload.get(key).and_then(Value::as_str))
}

fn exact_candidate_name_matches(payload: &Map<String, Value>, term: &str) -> bool {
    ["name", "qualifiedName", "symbolIdentity", "ownerIdentity"]
        .into_iter()
        .filter_map(|key| payload.get(key).and_then(Value::as_str))
        .any(|identity| identity == term)
}

fn exact_query_name_match(payload: &Map<String, Value>, terms: &BTreeSet<String>) -> bool {
    ["name", "qualifiedName"]
        .into_iter()
        .filter_map(|key| payload.get(key).and_then(Value::as_str))
        .map(str::to_lowercase)
        .any(|name| terms.contains(&name))
}

fn validate_file_selector(file: &str) -> Result<(), ClewError> {
    if file.is_empty()
        || file.len() > 4096
        || file.contains("://")
        || file.starts_with('/')
        || file.contains('\0')
        || file
            .split('/')
            .any(|component| component.is_empty() || matches!(component, "." | ".."))
    {
        return Err(invalid(
            "navigation file selector is not repository-relative",
        ));
    }
    Ok(())
}

fn candidate_relevance(
    payload: &Map<String, Value>,
    sources: &[Value],
    terms: &BTreeSet<String>,
) -> (usize, usize, usize) {
    let name_text = ["name", "qualifiedName", "symbolIdentity", "ownerIdentity"]
        .into_iter()
        .filter_map(|key| payload.get(key).and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n")
        .to_lowercase();
    let source_text = exact_declaration_text(sources, payload)
        .unwrap_or_default()
        .to_lowercase();
    let mut term_coverage = 0usize;
    let mut name_coverage = 0usize;
    let mut occurrences = 0usize;
    for term in terms {
        let name_occurrences = name_text.matches(term).count();
        let source_occurrences = source_text.matches(term).count();
        if name_occurrences + source_occurrences > 0 {
            term_coverage += 1;
        }
        if name_occurrences > 0 {
            name_coverage += 1;
        }
        occurrences = occurrences.saturating_add(name_occurrences + source_occurrences);
    }
    (term_coverage, name_coverage, occurrences)
}

fn candidate_authority_term_hits(
    payload: &Map<String, Value>,
    sources: &[Value],
    terms: &BTreeSet<String>,
) -> BTreeSet<String> {
    let identity_text = ["name", "qualifiedName", "symbolIdentity", "ownerIdentity"]
        .into_iter()
        .filter_map(|key| payload.get(key).and_then(Value::as_str))
        .collect::<Vec<_>>();
    let declaration_text = boundary_safe_declaration_text(sources, payload);
    let mut authority_text = identity_text;
    if let Some(declaration_text) = declaration_text.as_deref() {
        authority_text.push(declaration_text);
    }
    let lexical_terms = crate::query_v2::normalize_index_terms(authority_text)
        .into_iter()
        .collect::<BTreeSet<_>>();
    terms
        .iter()
        .filter(|term| lexical_terms.contains(term.as_str()))
        .cloned()
        .collect()
}

fn candidate_matches_declared_identifier(
    payload: &Map<String, Value>,
    decision_identifier: &str,
) -> bool {
    ["name", "qualifiedName", "symbolIdentity"]
        .into_iter()
        .filter_map(|key| payload.get(key).and_then(Value::as_str))
        .any(|candidate| candidate == decision_identifier)
}

fn source_window_relevance(
    sources: &[Value],
    payload: &Map<String, Value>,
    terms: &BTreeSet<String>,
) -> (usize, usize) {
    let Some(source_text) = exact_source(sources, payload)
        .and_then(|source| source.window.get("text"))
        .and_then(Value::as_str)
        .map(str::to_lowercase)
    else {
        return (0, 0);
    };
    terms.iter().fold((0usize, 0usize), |score, term| {
        let occurrences = source_text.matches(term).count();
        (
            score.0 + usize::from(occurrences > 0),
            score.1.saturating_add(occurrences),
        )
    })
}

fn source_window_term_hits(
    sources: &[Value],
    payload: &Map<String, Value>,
    terms: &BTreeSet<String>,
) -> Option<((u64, u64), BTreeSet<String>)> {
    let source = exact_source(sources, payload)?;
    let start_line = source.window.get("startLine").and_then(Value::as_u64)?;
    let end_line = source.window.get("endLine").and_then(Value::as_u64)?;
    let text = source
        .window
        .get("text")
        .and_then(Value::as_str)?
        .to_lowercase();
    let hits = terms
        .iter()
        .filter(|term| text.contains(term.as_str()))
        .cloned()
        .collect();
    Some(((start_line, end_line), hits))
}

fn exact_declaration_text(sources: &[Value], payload: &Map<String, Value>) -> Option<String> {
    let source = exact_source(sources, payload)?;
    let window_start = source.window.get("startLine").and_then(Value::as_u64)?;
    let declaration_start = payload.get("startLine").and_then(Value::as_u64)?;
    let declaration_end = payload.get("endLine").and_then(Value::as_u64)?;
    let start = usize::try_from(declaration_start.checked_sub(window_start)?).ok()?;
    let count = usize::try_from(
        declaration_end
            .checked_sub(declaration_start)?
            .checked_add(1)?,
    )
    .ok()?;
    let lines = source
        .window
        .get("text")
        .and_then(Value::as_str)?
        .split('\n')
        .collect::<Vec<_>>();
    let end = start.checked_add(count)?;
    (end <= lines.len()).then(|| lines[start..end].join("\n"))
}

fn boundary_safe_declaration_text(
    sources: &[Value],
    payload: &Map<String, Value>,
) -> Option<String> {
    let source = exact_source(sources, payload)?;
    let text = source.window.get("text").and_then(Value::as_str)?;
    if source
        .row
        .get("completeFile")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        && source.window.get("startLine").and_then(Value::as_u64) == Some(1)
    {
        let start = payload
            .get("start")
            .or_else(|| payload.get("rangeStart"))
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok());
        let end = payload
            .get("end")
            .or_else(|| payload.get("rangeEnd"))
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok());
        if let (Some(start), Some(end)) = (start, end)
            && start < end
            && end <= text.len()
            && text.is_char_boundary(start)
            && text.is_char_boundary(end)
        {
            return Some(text[start..end].to_owned());
        }
    }

    let declaration = exact_declaration_text(sources, payload)?;
    let lines = declaration.split('\n').collect::<Vec<_>>();
    (lines.len() > 2).then(|| lines[1..lines.len() - 1].join("\n"))
}

fn retained_context(context: &ContextObject) -> Result<&Value, ClewError> {
    context
        .evidence
        .get("context")
        .ok_or_else(|| invalid("context has no retained navigation evidence"))
}

fn retained_query_truncated(context: &ContextObject) -> bool {
    context
        .evidence
        .pointer("/queryContext/truncated")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn candidate_handle(compilation: &str, fact_key: &str) -> Result<String, ClewError> {
    let digest = canonical::hash(&json!({
        "compilation":compilation,
        "factKey":fact_key,
    }))
    .map_err(internal)?;
    let hex = digest
        .strip_prefix("sha256:")
        .ok_or_else(|| internal("candidate digest has no sha256 prefix"))?;
    Ok(format!("c:{}", &hex[..16]))
}

struct ExactSource<'a> {
    row: &'a Value,
    window: &'a Value,
}

fn exact_source<'a>(sources: &'a [Value], payload: &Map<String, Value>) -> Option<ExactSource<'a>> {
    let file = payload.get("file").and_then(Value::as_str)?;
    let declaration_start = payload.get("startLine").and_then(Value::as_u64)?;
    let declaration_end = payload.get("endLine").and_then(Value::as_u64)?;
    sources
        .iter()
        .filter(|row| row.get("fileId").and_then(Value::as_str) == Some(file))
        .flat_map(|row| {
            row.get("windows")
                .and_then(Value::as_array)
                .filter(|windows| !windows.is_empty())
                .map_or_else(
                    || vec![ExactSource { row, window: row }],
                    |windows| {
                        windows
                            .iter()
                            .map(|window| ExactSource { row, window })
                            .collect()
                    },
                )
        })
        .filter(|source| {
            let start = source.window.get("startLine").and_then(Value::as_u64);
            let end = source.window.get("endLine").and_then(Value::as_u64);
            start.is_some_and(|start| start <= declaration_start)
                && end.is_some_and(|end| declaration_end <= end)
        })
        .min_by_key(|source| {
            let start = source.window["startLine"].as_u64().unwrap_or(0);
            let end = source.window["endLine"].as_u64().unwrap_or(u64::MAX);
            (end.saturating_sub(start), start)
        })
}

fn exact_source_preview(sources: &[Value], payload: &Map<String, Value>) -> Value {
    let Some(source) = exact_source(sources, payload) else {
        return json!({"status":"UNAVAILABLE","reason":"NO_EXACT_DECLARATION_WINDOW"});
    };
    let Some(declaration_start) = payload.get("startLine").and_then(Value::as_u64) else {
        return json!({"status":"UNAVAILABLE","reason":"NO_EXACT_LINE_RANGE"});
    };
    let Some(window_start) = source.window.get("startLine").and_then(Value::as_u64) else {
        return json!({"status":"UNAVAILABLE","reason":"SOURCE_WINDOW_HAS_NO_LINE_RANGE"});
    };
    let Some(text) = source.window.get("text").and_then(Value::as_str) else {
        return json!({"status":"UNAVAILABLE","reason":"SOURCE_WINDOW_HAS_NO_TEXT"});
    };
    let Some(line) = usize::try_from(declaration_start.saturating_sub(window_start))
        .ok()
        .and_then(|offset| text.split('\n').nth(offset))
    else {
        return json!({"status":"UNAVAILABLE","reason":"DECLARATION_LINE_NOT_RETAINED"});
    };
    let (text, line_truncated) = bounded_line_prefix(line, 512);
    let content_digest = source
        .row
        .pointer("/contentRef/digest")
        .and_then(Value::as_str);
    json!({
        "status":"EXACT",
        "authority":"EXACT_SNAPSHOT_TEXT",
        "contentDigest":content_digest,
        "startLine":declaration_start,
        "endLine":declaration_start,
        "text":text,
        "truncated":line_truncated
            || payload.get("endLine").and_then(Value::as_u64) != Some(declaration_start),
    })
}

fn exact_source_detail(sources: &[Value], payload: &Map<String, Value>) -> Value {
    let Some(source) = exact_source(sources, payload) else {
        return json!({"status":"UNSUPPORTED","reason":"NO_EXACT_DECLARATION_WINDOW"});
    };
    json!({
        "status":"SUPPORTED",
        "authority":"EXACT_SNAPSHOT_TEXT",
        "fileId":source.row.get("fileId"),
        "contentRef":source.row.get("contentRef"),
        "completeFile":source.row.get("completeFile").and_then(Value::as_bool).unwrap_or(false),
        "windows":[source.window],
    })
}

fn term_anchors(sources: &[Value], terms: &[String]) -> Vec<Value> {
    #[derive(Clone)]
    struct AnchorLine {
        file: String,
        line: u64,
        content_digest: Option<String>,
        text: String,
        matches: BTreeMap<String, usize>,
    }

    let normalized_terms = terms
        .iter()
        .filter_map(|term| {
            let lowered = term.to_lowercase();
            (!lowered.is_empty()).then(|| (term.clone(), lowered))
        })
        .collect::<BTreeMap<_, _>>();
    let mut lines = BTreeMap::<(String, u64), AnchorLine>::new();
    for row in sources {
        let Some(file) = row.get("fileId").and_then(Value::as_str) else {
            continue;
        };
        let content_digest = row
            .pointer("/contentRef/digest")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let windows = row
            .get("windows")
            .and_then(Value::as_array)
            .filter(|windows| !windows.is_empty())
            .map_or_else(|| vec![row], |windows| windows.iter().collect());
        for window in windows {
            let Some(start) = window.get("startLine").and_then(Value::as_u64) else {
                continue;
            };
            let Some(text) = window.get("text").and_then(Value::as_str) else {
                continue;
            };
            for (index, text) in text.split('\n').enumerate() {
                let lowered_line = text.to_lowercase();
                let matches = normalized_terms
                    .iter()
                    .filter_map(|(term, lowered_term)| {
                        let occurrences = lowered_line.matches(lowered_term).count();
                        (occurrences > 0).then(|| (term.clone(), occurrences))
                    })
                    .collect::<BTreeMap<_, _>>();
                if matches.is_empty() {
                    continue;
                }
                let line = start.saturating_add(index as u64);
                lines
                    .entry((file.to_owned(), line))
                    .or_insert_with(|| AnchorLine {
                        file: file.to_owned(),
                        line,
                        content_digest: content_digest.clone(),
                        text: text.to_owned(),
                        matches,
                    });
            }
        }
    }

    let mut uncovered = normalized_terms.keys().cloned().collect::<BTreeSet<_>>();
    let mut selected = Vec::new();
    let mut used = BTreeSet::new();
    while !uncovered.is_empty() && selected.len() < MAX_TERM_ANCHORS {
        let mut candidates = lines
            .values()
            .filter(|line| !used.contains(&(line.file.clone(), line.line)))
            .map(|line| {
                let uncovered_coverage = line
                    .matches
                    .keys()
                    .filter(|term| uncovered.contains(*term))
                    .count();
                let uncovered_occurrences = line
                    .matches
                    .iter()
                    .filter(|(term, _)| uncovered.contains(*term))
                    .map(|(_, occurrences)| occurrences)
                    .sum::<usize>();
                (line, uncovered_coverage, uncovered_occurrences)
            })
            .filter(|(_, coverage, _)| *coverage > 0)
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| {
            right
                .1
                .cmp(&left.1)
                .then_with(|| right.2.cmp(&left.2))
                .then_with(|| right.0.matches.len().cmp(&left.0.matches.len()))
                .then_with(|| left.0.file.cmp(&right.0.file))
                .then_with(|| left.0.line.cmp(&right.0.line))
        });
        let Some((line, _, _)) = candidates.first() else {
            break;
        };
        let line = (*line).clone();
        for term in line.matches.keys() {
            uncovered.remove(term);
        }
        used.insert((line.file.clone(), line.line));
        selected.push(line);
    }

    if !selected.is_empty() && selected.len() < MAX_TERM_ANCHORS {
        let mut denser = lines
            .values()
            .filter(|line| !used.contains(&(line.file.clone(), line.line)))
            .filter_map(|line| {
                let score = (line.matches.len(), line.matches.values().sum::<usize>());
                let selected_score = selected
                    .iter()
                    .map(|selected| {
                        line.matches.keys().fold((0usize, 0usize), |score, term| {
                            selected.matches.get(term).map_or(score, |occurrences| {
                                (score.0 + 1, score.1.saturating_add(*occurrences))
                            })
                        })
                    })
                    .max()
                    .unwrap_or((0, 0));
                (score > selected_score).then_some((line, score))
            })
            .collect::<Vec<_>>();
        denser.sort_by(|left, right| {
            right
                .1
                .cmp(&left.1)
                .then_with(|| left.0.file.cmp(&right.0.file))
                .then_with(|| left.0.line.cmp(&right.0.line))
        });
        if let Some((line, _)) = denser.first() {
            selected.push((*line).clone());
        }
    }

    selected
        .into_iter()
        .map(|line| {
            let (text, truncated) = bounded_line_prefix(&line.text, 512);
            json!({
                "authority":"EXACT_SNAPSHOT_TEXT_LEXICAL",
                "matchedTerms":line.matches.keys().collect::<Vec<_>>(),
                "file":line.file,
                "line":line.line,
                "contentDigest":line.content_digest,
                "text":text,
                "truncated":truncated,
            })
        })
        .collect()
}

fn bounded_line_prefix(line: &str, limit: usize) -> (&str, bool) {
    if line.len() <= limit {
        return (line, false);
    }
    let mut end = limit;
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    (&line[..end], true)
}

fn bounded_optional_string(value: Option<&str>, limit: usize) -> (Option<&str>, bool) {
    value.map_or((None, false), |value| {
        let (bounded, truncated) = bounded_line_prefix(value, limit);
        (Some(bounded), truncated)
    })
}

fn supported(authority: &str) -> Value {
    json!({"status":"SUPPORTED","authority":authority})
}

fn unsupported(reason: &str) -> Value {
    json!({"status":"UNSUPPORTED","reason":reason})
}

fn required_string<'a>(value: &'a Value, key: &str) -> Result<&'a str, ClewError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| invalid(format!("navigation match has no {key}")))
}

fn required_payload_string<'a>(
    payload: &'a Map<String, Value>,
    key: &str,
) -> Result<&'a str, ClewError> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| invalid(format!("navigation payload has no {key}")))
}

fn required_payload_u64(payload: &Map<String, Value>, key: &str) -> Result<u64, ClewError> {
    payload
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| invalid(format!("navigation payload has no {key}")))
}

fn required_value_string<'a>(value: &'a Value, key: &str) -> Result<&'a str, ClewError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| invalid(format!("navigation value has no {key}")))
}

fn required_value_u64(value: &Value, key: &str) -> Result<u64, ClewError> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| invalid(format!("navigation value has no {key}")))
}

fn invalid(message: impl Into<String>) -> ClewError {
    ClewError::new(ErrorCode::InvalidInput, message)
}

fn internal(error: impl std::fmt::Display) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn projection() -> Value {
        json!({
            "matches":[
                {
                    "compilation":"cargo:Cargo.toml#demo#lib#demo",
                    "factKey":"fact:target",
                    "domainUri":"analysis:syntax",
                    "payloadRef":{"digest":"sha256:target"},
                    "payload":{
                        "kind":"declaration",
                        "name":"Target",
                        "declarationKind":"function",
                        "symbolIdentity":"symbol:Target",
                        "file":"src/lib.rs",
                        "startLine":10,
                        "endLine":14,
                        "rangeStart":100,
                        "rangeEnd":180
                    }
                },
                {
                    "compilation":"cargo:Cargo.toml#demo#lib#demo",
                    "factKey":"fact:namesake-call",
                    "domainUri":"analysis:syntax",
                    "payload":{"kind":"call","callee":"Target","file":"src/other.rs"}
                }
            ],
            "sources":[
                {
                    "fileId":"src/lib.rs",
                    "contentRef":{"digest":"sha256:wide"},
                    "startLine":1,
                    "endLine":40,
                    "text":"1\n2\n3\n4\n5\n6\n7\n8\n9\nwide-target\n11\n12\n13\n14\n15\n16\n17\n18\n19\n20\n21\n22\n23\n24\n25\n26\n27\n28\n29\n30\n31\n32\n33\n34\n35\n36\n37\n38\n39\n40"
                },
                {
                    "fileId":"src/lib.rs",
                    "contentRef":{"digest":"sha256:bounded"},
                    "startLine":8,
                    "endLine":16,
                    "text":"8\n9\nbounded\n11\n12\n13\n14\n15\n16"
                }
            ],
            "completeness":{"support":"SUPPORTED","certainty":"UNSURE"},
            "truncated":false
        })
    }

    #[test]
    fn builds_one_fact_bound_candidate_and_does_not_promote_namesakes() {
        let result = assemble(
            "session:test",
            "context:test",
            "sha256:evidence",
            NAV_QUERY_INTENT,
            &["Target".into()],
            &projection(),
            false,
            &[],
        )
        .unwrap();
        assert_eq!(result["candidates"].as_array().unwrap().len(), 1);
        assert!(
            result["candidates"][0]["candidateId"]
                .as_str()
                .unwrap()
                .starts_with("c:")
        );
        assert_eq!(result["candidates"][0]["preview"]["text"], "bounded");
        assert_eq!(result["facets"]["callers"]["status"], "NOT_REQUESTED");
        assert_eq!(result["intent"], NAV_QUERY_INTENT);
        assert!(result["candidates"][0].get("fact").is_none());
        assert!(result["candidates"][0].get("source").is_none());
        assert_eq!(
            result["nextAction"],
            json!({
                "detail":"nav expand --session <sessionId> --from <contextId> --candidate <candidateId> [--candidate <candidateId> ...] [--source] [--facet callers|callees|tests]",
                "refine":"nav expand --session <sessionId> --from <contextId> --term <additional-term>",
                "exactSource":"nav expand --session <sessionId> --from <contextId> --term <exact-identifier> --file <repository-relative-file> --source",
            })
        );
        assert_eq!(
            result["nextActions"],
            json!({
                "schema":NAV_ACTION_SCHEMA,
                "candidateSource":{
                    "kind":"CANDIDATE_SOURCE",
                    "maxCandidates":3,
                    "includeSource":true,
                    "includeFacet":false,
                },
                "exactSource":{
                    "kind":"EXACT_FILE_SOURCE",
                    "maxTerms":3,
                    "sameFileRequired":true,
                    "includeSource":true,
                },
                "facet":{
                    "kind":"CANDIDATE_FACET",
                    "allowedValues":["callers","callees","tests"],
                    "includeSource":false,
                    "explicitRelationOnly":true,
                },
                "refine":{
                    "kind":"TERM_REFINEMENT",
                    "includeSource":false,
                },
            })
        );
    }

    #[test]
    fn range_based_syntax_candidate_builds_a_compact_bound_source_card() {
        let retained = projection();
        let mut authority = context(
            "context:range-source-card",
            None,
            "sha256:evidence",
            retained,
        );
        authority.terms = vec!["Target".into()];
        let mut result = query_with_decision_identifier(&authority, &[], Some("Target")).unwrap();
        let candidate_id = result["candidates"][0]["candidateId"]
            .as_str()
            .unwrap()
            .to_owned();
        let source = source_envelope_by_candidate(&authority, &candidate_id).unwrap();
        result
            .as_object_mut()
            .unwrap()
            .insert("decisionSource".into(), source);

        let card = agent_card(&result).unwrap();
        assert_eq!(card["nextActions"], result["nextActions"]);
        assert_eq!(card["decisionSource"]["source"]["status"], "SUPPORTED");
        assert_eq!(
            card["decisionSource"]["source"]["contentDigest"],
            "sha256:bounded"
        );
        assert_eq!(
            card["decisionSource"]["sourceBindingCount"],
            json!({"eligible":1,"returned":1,"omitted":0})
        );
        assert_eq!(
            card["decisionSource"]["sourceBindings"][0]["candidateId"],
            candidate_id
        );
        assert_eq!(
            card["decisionSource"]["sourceDelivery"],
            json!({
                "status":"RETURNED",
                "candidateIds":[candidate_id],
                "reuse":"CURRENT_RESULT",
                "repeatSameRequest":false,
            })
        );
    }

    #[test]
    fn kotlin_descriptor_lines_enable_exact_preview_and_decision_source() {
        let retained = json!({
            "matches":[{
                "compilation":":/main",
                "factKey":"descriptor:engine-policy",
                "domainUri":"analysis:kotlin",
                "payloadRef":{"digest":"sha256:descriptor"},
                "payload":{
                    "schema":"declaration-descriptor/0.1",
                    "declarationKind":"FUNCTION",
                    "symbolIdentity":"callable:dev/semanticthread/worker/kotlinEngineCompatibilityDecision#jvm:()V",
                    "file":"Worker.kt",
                    "start":20,
                    "end":100,
                    "startLine":2,
                    "endLine":4,
                    "lineProvenance":"UTF8_BYTE_RANGE_OVER_COMPILATION_SOURCE"
                }
            }],
            "sources":[{
                "fileId":"Worker.kt",
                "contentRef":{"digest":"sha256:kotlin-source"},
                "startLine":1,
                "endLine":5,
                "windows":[{
                    "startLine":1,
                    "endLine":5,
                    "text":"package demo\nfun kotlinEngineCompatibilityDecision() {\n  check(true)\n}\n"
                }],
                "completeFile":false
            }],
            "completeness":{"support":"SUPPORTED","certainty":"UNSURE"},
            "truncated":false
        });
        let mut authority = context(
            "context:kotlin-lines",
            None,
            "sha256:kotlin-evidence",
            retained,
        );
        authority.terms = vec!["kotlinEngineCompatibilityDecision".into()];

        let cards = query(&authority, &[]).unwrap();
        assert_eq!(cards["candidates"][0]["preview"]["status"], "EXACT");
        assert_eq!(cards["candidates"][0]["location"]["startLine"], 2);
        let candidate = cards["candidates"][0]["candidateId"].as_str().unwrap();
        let source = source_envelope_by_candidate(&authority, candidate).unwrap();
        assert_eq!(source["source"]["status"], "SUPPORTED");
        assert_eq!(
            source["source"]["contentRef"]["digest"],
            "sha256:kotlin-source"
        );
    }

    #[test]
    fn decision_card_is_compact_and_detail_preserves_exact_fact_and_window() {
        let retained = json!({
            "matches":[{
                "compilation":"cargo:Cargo.toml#demo#lib#demo",
                "factKey":"fact:target",
                "domainUri":"analysis:syntax",
                "payloadRef":{"digest":"sha256:fact"},
                "payload":{
                    "kind":"declaration",
                    "name":"Target",
                    "declarationKind":"function",
                    "symbolIdentity":"symbol:Target",
                    "file":"src/lib.rs",
                    "startLine":10,
                    "endLine":12,
                    "descriptor":{"resolution":"EXACT","shape":"()V"}
                }
            }],
            "sources":[{
                "fileId":"src/lib.rs",
                "contentRef":{"digest":"sha256:source"},
                "startLine":1,
                "endLine":12,
                "text":"unrelated\nTARGET_MARKER\nbody",
                "windows":[
                    {"startLine":1,"endLine":1,"text":"unrelated"},
                    {"startLine":10,"endLine":12,"text":"TARGET_MARKER\nbody\n}"}
                ],
                "completeFile":false
            }],
            "completeness":{"certainty":"VERIFIED"},
            "truncated":false
        });
        let authority = context("context:detail", None, "sha256:evidence", retained);
        let cards = query(&authority, &[]).unwrap();
        let card = &cards["candidates"][0];
        assert!(card.get("fact").is_none());
        assert!(card.get("source").is_none());
        assert_eq!(card["preview"]["text"], "TARGET_MARKER");
        let candidate_id = card["candidateId"].as_str().unwrap();
        let selected = detail(&authority, &[candidate_id.into()], true, &[]).unwrap();
        assert_eq!(
            selected["candidates"][0]["fact"]["payload"]["descriptor"],
            json!({"resolution":"EXACT","shape":"()V"})
        );
        assert_eq!(
            selected["candidates"][0]["source"]["windows"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert!(selected["candidates"][0]["source"].get("text").is_none());
        assert_eq!(
            selected["candidates"][0]["sourceDelivery"],
            json!({
                "status":"RETURNED",
                "reuse":"CURRENT_RESULT",
                "repeatSameRequest":false,
            })
        );
        let rendered = canonical::compact(&selected).unwrap();
        assert_eq!(rendered.matches("TARGET_MARKER").count(), 1);
    }

    #[test]
    fn source_delivery_distinguishes_returned_not_requested_and_unsupported() {
        assert_eq!(
            source_delivery(&json!({"status":"SUPPORTED"})).unwrap(),
            json!({
                "status":"RETURNED",
                "reuse":"CURRENT_RESULT",
                "repeatSameRequest":false,
            })
        );
        assert_eq!(
            source_delivery(&json!({"status":"NOT_REQUESTED"})).unwrap(),
            json!({
                "status":"NOT_RETURNED",
                "requestAction":"candidateSource",
            })
        );
        assert_eq!(
            source_delivery(&json!({
                "status":"UNSUPPORTED",
                "reason":"NO_EXACT_DECLARATION_WINDOW",
            }))
            .unwrap(),
            json!({
                "status":"UNAVAILABLE",
                "reason":"NO_EXACT_DECLARATION_WINDOW",
                "repeatSameRequest":false,
            })
        );
        assert_eq!(
            source_delivery(&json!({"status":"MAYBE"}))
                .unwrap_err()
                .code,
            ErrorCode::InvalidInput
        );
    }

    #[test]
    fn agent_card_keeps_decision_authority_and_removes_redundant_envelope_fields() {
        let result = json!({
            "schema":NAV_RESULT_SCHEMA,
            "sessionId":"session:one",
            "contextId":"context:one",
            "evidenceDigest":"sha256:evidence",
            "terms":["restart","service"],
            "candidates":[
                {
                    "candidateId":"c:one",
                    "candidateKey":{"compilation":"tsconfig:app.json","factKey":"fact:one"},
                    "displayName":"restartService",
                    "declarationKind":"FUNCTION",
                    "symbolIdentity":"ts:src/service.ts#function:restartService@10-80",
                    "location":{"file":"src/service.ts","startLine":2,"endLine":5,"start":10,"end":80},
                    "preview":{"status":"EXACT","authority":"EXACT_SNAPSHOT_TEXT","contentDigest":"sha256:source","startLine":2,"endLine":2,"text":"function restartService() {","truncated":true}
                },
                {
                    "candidateId":"c:two",
                    "candidateKey":{"compilation":"tsconfig:app.json","factKey":"fact:two"},
                    "displayName":"ServiceActions",
                    "declarationKind":"FUNCTION",
                    "symbolIdentity":"ts:src/actions.ts#function:ServiceActions@10-80",
                    "location":{"file":"src/actions.ts","startLine":8,"endLine":12,"start":10,"end":80},
                    "preview":{"status":"EXACT","authority":"EXACT_SNAPSHOT_TEXT","contentDigest":"sha256:actions","startLine":8,"endLine":8,"text":"function ServiceActions() {","truncated":true}
                }
            ],
            "candidateCount":{"returned":2,"total":4,"omitted":2},
            "decisionAuthority":{
                "schema":NAV_DECISION_AUTHORITY_SCHEMA,
                "status":"SUPPORTED",
                "classification":"UNIQUE_EXACT_IDENTIFIER_FULL_COVERAGE",
                "candidateId":"c:one"
            },
            "decisionSource":{
                "candidateId":"c:one",
                "selectionAuthority":"TOP_CANDIDATE",
                "sourceBindingCount":{"eligible":2,"returned":2,"omitted":0},
                "sourceBindings":[
                    {"candidateId":"c:one","candidateKey":{"compilation":"tsconfig:app.json","factKey":"fact:one"},"displayName":"restartService","declarationStartLine":2,"declarationEndLine":5,"windowIndex":0},
                    {"candidateId":"c:helper","candidateKey":{"compilation":"tsconfig:app.json","factKey":"fact:helper"},"displayName":"request","declarationStartLine":10,"declarationEndLine":12,"windowIndex":1}
                ],
                "source":{
                    "status":"SUPPORTED",
                    "authority":"EXACT_SNAPSHOT_TEXT",
                    "fileId":"src/service.ts",
                    "contentRef":{"schema":"codeclew-cas-object/2.0","objectSchema":"source","digest":"sha256:source","size":100},
                    "completeFile":false,
                    "truncated":false,
                    "windows":[
                        {"startLine":2,"endLine":5,"text":"function restartService() {\n  return request()\n}"},
                        {"startLine":10,"endLine":12,"text":"function request() {\n  return fetch()\n}"}
                    ]
                }
            },
            "termAnchors":[
                {"authority":"EXACT_SNAPSHOT_TEXT_LEXICAL","file":"src/service.ts","line":3,"text":"return request()","contentDigest":"sha256:source","matchedTerms":["service"]},
                {"authority":"EXACT_SNAPSHOT_TEXT_LEXICAL","file":"src/service.test.ts","line":20,"text":"it('restarts a service')","contentDigest":"sha256:test","matchedTerms":["restart","service"]}
            ],
            "facets":{"callers":{"status":"NOT_REQUESTED"}},
            "completeness":{"status":"CONDITIONAL_TASK","coverage":"PARTIAL","certainty":"UNSURE"},
            "queryCoverageTruncated":false,
            "candidateListTruncated":true,
            "truncated":true,
            "nextAction":{"detail":"nav expand ..."},
        });

        let card = agent_card(&result).unwrap();

        assert_eq!(card["schema"], NAV_AGENT_CARD_SCHEMA);
        assert_eq!(
            card["decisionAuthority"]["classification"],
            "UNIQUE_EXACT_IDENTIFIER_FULL_COVERAGE"
        );
        assert_eq!(card["candidates"][0]["candidateId"], "c:one");
        assert_eq!(card["candidates"][1]["preview"]["status"], "EXACT");
        assert_eq!(
            card["candidates"][1]["preview"]["authority"],
            "EXACT_SNAPSHOT_TEXT"
        );
        assert_eq!(
            card["candidates"][1]["preview"]["contentDigest"],
            "sha256:actions"
        );
        assert!(card["candidates"][0].get("candidateKey").is_none());
        assert!(card["candidates"][0].get("symbolIdentity").is_none());
        assert_eq!(
            card["decisionSource"]["source"]["contentDigest"],
            "sha256:source"
        );
        assert!(card["decisionSource"]["source"].get("contentRef").is_none());
        assert_eq!(
            card["decisionSource"]["sourceBindings"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            card["decisionSource"]["sourceBindings"][1]["windowIndex"],
            1
        );
        assert_eq!(
            card["decisionSource"]["sourceDelivery"]["candidateIds"],
            json!(["c:one", "c:helper"])
        );
        assert!(
            card["decisionSource"]["sourceBindings"][0]
                .get("candidateKey")
                .is_none()
        );
        assert_eq!(card["supportingAnchors"].as_array().unwrap().len(), 1);
        assert_eq!(card["supportingAnchors"][0]["file"], "src/service.test.ts");
        assert!(card.get("facets").is_none());
        assert!(canonical::bytes(&card).unwrap().len() < canonical::bytes(&result).unwrap().len());

        let mut conflicting = result.clone();
        conflicting["decisionAuthority"] = json!({
            "schema":NAV_DECISION_AUTHORITY_SCHEMA,
            "status":"ABSTAIN",
            "classification":"NO_DECLARED_DECISION_IDENTIFIER"
        });
        assert_eq!(
            agent_card(&conflicting).unwrap_err().code,
            ErrorCode::InvalidInput
        );
    }

    #[test]
    fn agent_card_preserves_unsupported_reason_and_rejects_unknown_statuses() {
        let mut result = json!({
            "schema":NAV_RESULT_SCHEMA,
            "sessionId":"session:one",
            "contextId":"context:one",
            "evidenceDigest":"sha256:evidence",
            "terms":["missing"],
            "candidates":[{
                "candidateId":"c:one",
                "displayName":"missing",
                "declarationKind":"FUNCTION",
                "location":{"file":"src/lib.ts","startLine":1,"endLine":1},
                "preview":{"status":"UNAVAILABLE","reason":"NO_EXACT_DECLARATION_WINDOW"}
            }],
            "candidateCount":{"returned":1,"total":1,"omitted":0},
            "decisionAuthority":{
                "schema":NAV_DECISION_AUTHORITY_SCHEMA,
                "status":"SUPPORTED",
                "classification":"UNIQUE_EXACT_IDENTIFIER_FULL_COVERAGE",
                "candidateId":"c:one"
            },
            "decisionSource":{
                "candidateId":"c:one",
                "source":{"status":"UNSUPPORTED","reason":"NO_EXACT_DECLARATION_WINDOW"}
            },
            "termAnchors":[],
            "completeness":{"status":"CONDITIONAL_TASK","coverage":"PARTIAL","certainty":"UNSURE"},
            "queryCoverageTruncated":false,
            "candidateListTruncated":false,
            "truncated":false,
            "nextAction":{"refine":"nav expand ..."},
        });

        let card = agent_card(&result).unwrap();
        assert_eq!(
            card["decisionSource"]["source"]["reason"],
            "NO_EXACT_DECLARATION_WINDOW"
        );
        assert_eq!(
            card["decisionSource"]["sourceDelivery"]["status"],
            "UNSUPPORTED"
        );
        assert!(!canonical::compact(&card).unwrap().contains(":null"));

        result["decisionSource"]["source"]["status"] = json!("UNAVAILABLE");
        let unavailable = agent_card(&result).unwrap();
        assert_eq!(
            unavailable["decisionSource"]["sourceDelivery"],
            json!({
                "status":"UNAVAILABLE",
                "reason":"NO_EXACT_DECLARATION_WINDOW",
                "repeatSameRequest":false,
            })
        );

        result["decisionSource"]["source"]["status"] = json!("MAYBE");
        assert_eq!(
            agent_card(&result).unwrap_err().code,
            ErrorCode::InvalidInput
        );
        result["decisionSource"]["source"]["status"] = json!("UNSUPPORTED");
        result["candidates"][0]["preview"]["status"] = json!("MAYBE");
        assert_eq!(
            agent_card(&result).unwrap_err().code,
            ErrorCode::InvalidInput
        );
    }

    #[test]
    fn candidate_source_does_not_materialize_an_unreturned_large_fact() {
        let retained = json!({
            "matches":[{
                "compilation":"cargo:demo",
                "factKey":"fact:large",
                "payload":{
                    "kind":"declaration",
                    "name":"Large",
                    "symbolIdentity":"symbol:Large",
                    "file":"src/lib.rs",
                    "startLine":1,
                    "endLine":1,
                    "opaque":"x".repeat(128 * 1024)
                }
            }],
            "sources":[{
                "fileId":"src/lib.rs",
                "contentRef":{"digest":"sha256:source"},
                "windows":[{"startLine":1,"endLine":1,"text":"fn Large() {}"}]
            }],
            "completeness":{},
            "truncated":false
        });
        let authority = context("context:large-source", None, "sha256:evidence", retained);
        let cards = query(&authority, &[]).unwrap();
        let candidate_id = cards["candidates"][0]["candidateId"].as_str().unwrap();
        let selected = source_envelope_by_candidate(&authority, candidate_id).unwrap();
        let rendered = canonical::compact(&selected).unwrap();
        assert!(rendered.len() < 2048);
        assert!(!rendered.contains("opaque"));
        assert_eq!(
            selected["sourceBindingCount"],
            json!({"eligible":1,"returned":1,"omitted":0})
        );
        assert_eq!(selected["source"]["windows"][0]["text"], "fn Large() {}");
        validate_stdout(&selected).unwrap();
    }

    #[test]
    fn decision_source_envelope_binds_three_exact_same_file_declarations() {
        let retained = json!({
            "matches":[
                {
                    "compilation":"tsconfig:app/tsconfig.json",
                    "factKey":"fact:request",
                    "payload":{
                        "kind":"DECLARATION","name":"request","symbolIdentity":"ts:request",
                        "file":"src/client.ts","start":300,"end":500,"startLine":65,"endLine":87
                    }
                },
                {
                    "compilation":"tsconfig:app/tsconfig.json",
                    "factKey":"fact:error",
                    "payload":{
                        "kind":"DECLARATION","name":"ApiError","symbolIdentity":"ts:ApiError",
                        "file":"src/client.ts","start":100,"end":160,"startLine":41,"endLine":48
                    }
                },
                {
                    "compilation":"tsconfig:app/tsconfig.json",
                    "factKey":"fact:parse",
                    "payload":{
                        "kind":"DECLARATION","name":"parseErrorPayload","symbolIdentity":"ts:parseErrorPayload",
                        "file":"src/client.ts","start":170,"end":290,"startLine":50,"endLine":63
                    }
                },
                {
                    "compilation":"tsconfig:app/tsconfig.json",
                    "factKey":"fact:other-file",
                    "payload":{
                        "kind":"DECLARATION","name":"ApiError","symbolIdentity":"ts:other:ApiError",
                        "file":"src/other.ts","start":1,"end":20,"startLine":1,"endLine":2
                    }
                }
            ],
            "sources":[
                {
                    "fileId":"src/client.ts",
                    "contentRef":{"digest":"sha256:client"},
                    "completeFile":false,
                    "windows":[
                        {"startLine":41,"endLine":48,"text":"class ApiError extends Error {}"},
                        {"startLine":50,"endLine":63,"text":"function parseErrorPayload() {}"},
                        {"startLine":65,"endLine":87,"text":"async function request() {}"}
                    ]
                },
                {
                    "fileId":"src/other.ts",
                    "contentRef":{"digest":"sha256:other"},
                    "completeFile":true,
                    "windows":[{"startLine":1,"endLine":2,"text":"class ApiError {}"}]
                }
            ],
            "completeness":{},
            "truncated":false
        });
        let mut authority = context(
            "context:typescript-source-envelope",
            None,
            "sha256:evidence",
            retained,
        );
        authority.terms = vec![
            "ApiError".into(),
            "parseErrorPayload".into(),
            "request".into(),
        ];
        let candidate_id = candidate_handle("tsconfig:app/tsconfig.json", "fact:request").unwrap();
        let selected = source_envelope_by_candidate(&authority, &candidate_id).unwrap();

        assert_eq!(selected["source"]["status"], "SUPPORTED");
        assert_eq!(selected["source"]["truncated"], false);
        assert_eq!(selected["source"]["contentRef"]["digest"], "sha256:client");
        assert_eq!(selected["source"]["windows"].as_array().unwrap().len(), 3);
        assert_eq!(selected["source"]["windows"][0]["startLine"], 41);
        assert_eq!(selected["source"]["windows"][2]["endLine"], 87);
        assert_eq!(selected["sourceBindings"].as_array().unwrap().len(), 3);
        assert_eq!(selected["sourceBindings"][0]["displayName"], "request");
        assert_eq!(selected["sourceBindings"][0]["windowIndex"], 2);
        assert!(selected["sourceBytes"].as_u64().unwrap() <= 16 * 1024);
        validate_stdout(&selected).unwrap();
    }

    #[test]
    fn decision_source_envelope_omits_a_distant_exact_name_without_new_coverage() {
        let retained = json!({
            "matches":[
                {
                    "compilation":"tsconfig:app/tsconfig.json",
                    "factKey":"fact:parse",
                    "payload":{
                        "kind":"DECLARATION","name":"parseErrorPayload","symbolIdentity":"ts:parse",
                        "file":"src/client.ts","start":100,"end":180,"startLine":50,"endLine":63
                    }
                },
                {
                    "compilation":"tsconfig:app/tsconfig.json",
                    "factKey":"fact:headers",
                    "payload":{
                        "kind":"DECLARATION","name":"headers","symbolIdentity":"ts:unrelated:headers",
                        "file":"src/client.ts","start":500,"end":510,"startLine":170,"endLine":170
                    }
                }
            ],
            "sources":[{
                "fileId":"src/client.ts",
                "contentRef":{"digest":"sha256:client"},
                "completeFile":false,
                "windows":[
                    {
                        "startLine":41,"endLine":87,
                        "text":"function parseErrorPayload(payload: string) { return payload }\nconst headers = new Headers()"
                    },
                    {
                        "startLine":162,"endLine":194,
                        "text":"const headers: Record<string, string> = {}"
                    }
                ]
            }],
            "completeness":{},"truncated":false
        });
        let mut authority = context("context:marginal-source", None, "sha256:evidence", retained);
        authority.terms = vec!["parseErrorPayload".into(), "headers".into()];
        let candidate_id = candidate_handle("tsconfig:app/tsconfig.json", "fact:parse").unwrap();
        let selected = source_envelope_by_candidate(&authority, &candidate_id).unwrap();

        assert_eq!(selected["source"]["windows"].as_array().unwrap().len(), 1);
        assert_eq!(selected["sourceBindingCount"]["eligible"], 2);
        assert_eq!(selected["sourceBindingCount"]["returned"], 1);
        assert_eq!(selected["sourceBindingCount"]["omitted"], 1);
        assert_eq!(selected["source"]["truncated"], true);
        validate_stdout(&selected).unwrap();
    }

    #[test]
    fn decision_source_envelope_deduplicates_windows_and_enforces_its_byte_bound() {
        let shared = "function first() {}\nfunction second() {}";
        let retained = json!({
            "matches":[
                {"compilation":"tsconfig:app/tsconfig.json","factKey":"fact:first","payload":{
                    "kind":"DECLARATION","name":"first","symbolIdentity":"ts:first","file":"src/app.ts",
                    "start":1,"end":20,"startLine":1,"endLine":1
                }},
                {"compilation":"tsconfig:app/tsconfig.json","factKey":"fact:second","payload":{
                    "kind":"DECLARATION","name":"second","symbolIdentity":"ts:second","file":"src/app.ts",
                    "start":21,"end":40,"startLine":2,"endLine":2
                }}
            ],
            "sources":[{"fileId":"src/app.ts","contentRef":{"digest":"sha256:app"},"completeFile":true,"windows":[{
                "startLine":1,"endLine":2,"text":shared
            }]}],
            "completeness":{},"truncated":false
        });
        let mut authority = context("context:dedupe", None, "sha256:evidence", retained);
        authority.terms = vec!["first".into(), "second".into()];
        let candidate_id = candidate_handle("tsconfig:app/tsconfig.json", "fact:first").unwrap();
        let selected = source_envelope_by_candidate(&authority, &candidate_id).unwrap();
        assert_eq!(selected["source"]["windows"].as_array().unwrap().len(), 1);
        assert_eq!(selected["sourceBindings"].as_array().unwrap().len(), 2);
        assert_eq!(selected["sourceBytes"], shared.len());

        authority.projection["sources"][0]["windows"][0]["text"] =
            json!("x".repeat(MAX_DECISION_SOURCE_BYTES + 1));
        authority.evidence["context"] = authority.projection.clone();
        let bounded = source_envelope_by_candidate(&authority, &candidate_id).unwrap();
        assert_eq!(bounded["source"]["status"], "UNSUPPORTED");
        assert_eq!(
            bounded["source"]["reason"],
            "SOURCE_ENVELOPE_BUDGET_EXCEEDED"
        );
        validate_stdout(&bounded).unwrap();
    }

    #[test]
    fn oversized_context_window_falls_back_to_exact_declaration_slices() {
        let oversized = format!(
            "function first() {{}}\n{}\nfunction second() {{}}",
            "x".repeat(MAX_DECISION_SOURCE_BYTES + 1)
        );
        let retained = json!({
            "matches":[
                {"compilation":"tsconfig:app/tsconfig.json","factKey":"fact:first","payload":{
                    "kind":"DECLARATION","name":"first","symbolIdentity":"ts:first","file":"src/app.ts",
                    "start":1,"end":20,"startLine":1,"endLine":1
                }},
                {"compilation":"tsconfig:app/tsconfig.json","factKey":"fact:second","payload":{
                    "kind":"DECLARATION","name":"second","symbolIdentity":"ts:second","file":"src/app.ts",
                    "start":21,"end":40,"startLine":3,"endLine":3
                }}
            ],
            "sources":[{"fileId":"src/app.ts","contentRef":{"digest":"sha256:app"},"completeFile":true,"windows":[{
                "startLine":1,"endLine":3,"text":oversized
            }]}],
            "completeness":{},"truncated":false
        });
        let mut authority = context(
            "context:oversized-context",
            None,
            "sha256:evidence",
            retained,
        );
        authority.terms = vec!["first".into(), "second".into()];
        let candidate_id = candidate_handle("tsconfig:app/tsconfig.json", "fact:first").unwrap();
        let selected = source_envelope_by_candidate(&authority, &candidate_id).unwrap();

        assert_eq!(selected["source"]["status"], "SUPPORTED");
        assert_eq!(selected["source"]["truncated"], true);
        assert_eq!(selected["source"]["completeFile"], false);
        assert_eq!(selected["source"]["windows"].as_array().unwrap().len(), 2);
        assert_eq!(
            selected["source"]["windows"][0]["text"],
            "function first() {}"
        );
        assert_eq!(
            selected["source"]["windows"][1]["text"],
            "function second() {}"
        );
        assert_eq!(selected["sourceBindings"].as_array().unwrap().len(), 2);
        assert!(selected["sourceBytes"].as_u64().unwrap() < MAX_DECISION_SOURCE_BYTES as u64);
        validate_stdout(&selected).unwrap();
    }

    #[test]
    fn decision_source_envelope_preserves_java_offset_only_abstention() {
        let retained = json!({
            "matches":[{
                "compilation":"gradle:main",
                "factKey":"java:declaration:Client",
                "payload":{
                    "kind":"DECLARATION",
                    "declarationKind":"CLASS",
                    "symbolIdentity":"java:Client",
                    "file":"src/main/java/Client.java",
                    "start":10,
                    "end":100
                }
            }],
            "sources":[{
                "fileId":"src/main/java/Client.java",
                "contentRef":{"digest":"sha256:java"},
                "windows":[{"startLine":1,"endLine":5,"text":"class Client {}"}]
            }],
            "completeness":{},
            "truncated":false
        });
        let authority = context("context:java-offsets", None, "sha256:evidence", retained);
        let candidate_id = candidate_handle("gradle:main", "java:declaration:Client").unwrap();

        assert_eq!(
            source_envelope_by_candidate(&authority, &candidate_id).unwrap(),
            source_by_candidate(&authority, &candidate_id).unwrap()
        );
        assert_eq!(
            source_envelope_by_candidate(&authority, &candidate_id).unwrap()["source"]["status"],
            "UNSUPPORTED"
        );
    }

    #[test]
    fn candidate_source_fails_closed_without_an_overlapping_window() {
        let retained = json!({
            "matches":[{
                "compilation":"cargo:demo",
                "factKey":"fact:gap",
                "payload":{
                    "kind":"declaration",
                    "name":"Gap",
                    "symbolIdentity":"symbol:Gap",
                    "file":"src/lib.rs",
                    "startLine":10,
                    "endLine":12
                }
            }],
            "sources":[{
                "fileId":"src/lib.rs",
                "contentRef":{"digest":"sha256:source"},
                "startLine":1,
                "endLine":20,
                "text":"first\nsecond",
                "windows":[
                    {"startLine":1,"endLine":5,"text":"first"},
                    {"startLine":15,"endLine":20,"text":"second"}
                ],
                "completeFile":false
            }],
            "completeness":{},
            "truncated":false
        });
        let authority = context("context:gap", None, "sha256:evidence", retained);
        let cards = query(&authority, &[]).unwrap();
        assert_eq!(
            cards["candidates"][0]["preview"]["reason"],
            "NO_EXACT_DECLARATION_WINDOW"
        );
        let selected = detail(
            &authority,
            &[cards["candidates"][0]["candidateId"]
                .as_str()
                .unwrap()
                .into()],
            true,
            &[],
        )
        .unwrap();
        assert_eq!(selected["candidates"][0]["source"]["status"], "UNSUPPORTED");
    }

    #[test]
    fn term_anchor_preserves_small_exact_supporting_evidence() {
        let sources = vec![json!({
            "fileId":"src/lib.rs",
            "contentRef":{"digest":"sha256:source"},
            "windows":[
                {
                    "startLine":10,
                    "endLine":10,
                    "text":"fn member() -> Completeness {"
                },
                {
                    "startLine":20,
                    "endLine":22,
                    "text":"let unrelated = true;\n\"memberCompleteness\": member_completeness,\nlet tail = true;"
                }
            ]
        })];
        let anchors = term_anchors(&sources, &["member".into(), "completeness".into()]);
        assert_eq!(anchors.len(), 1);
        assert_eq!(anchors[0]["file"], "src/lib.rs");
        assert_eq!(anchors[0]["line"], 21);
        assert_eq!(
            anchors[0]["text"],
            "\"memberCompleteness\": member_completeness,"
        );
        assert_eq!(anchors[0]["contentDigest"], "sha256:source");
        assert_eq!(
            anchors[0]["matchedTerms"],
            json!(["completeness", "member"])
        );
        assert_eq!(anchors[0]["authority"], "EXACT_SNAPSHOT_TEXT_LEXICAL");
        assert_eq!(anchors[0]["truncated"], false);
    }

    #[test]
    fn term_anchor_adds_one_denser_subset_after_covering_every_term() {
        let sources = vec![json!({
            "fileId":"src/lib.rs",
            "contentRef":{"digest":"sha256:source"},
            "windows":[{
                "startLine":10,
                "endLine":12,
                "text":"let member_unmatched = completeness;\nfn global_member_unmatched() {}\n\"memberCompleteness\": member_completeness,"
            }]
        })];
        let anchors = term_anchors(
            &sources,
            &[
                "unmatched".into(),
                "global".into(),
                "member".into(),
                "completeness".into(),
            ],
        );
        assert_eq!(anchors.len(), 3);
        assert_eq!(anchors[2]["line"], 12);
        assert_eq!(
            anchors[2]["text"],
            "\"memberCompleteness\": member_completeness,"
        );
        assert_eq!(
            anchors[2]["matchedTerms"],
            json!(["completeness", "member"])
        );
    }

    #[test]
    fn direct_reference_selection_uses_query_overlap_without_claiming_resolution() {
        let retained = json!({
            "matches":[{
                "compilation":"cargo:demo",
                "factKey":"fact:test",
                "payload":{
                    "kind":"declaration",
                    "name":"aggregate_unmatched_terms_are_global",
                    "declarationKind":"function",
                    "symbolIdentity":"symbol:test",
                    "file":"src/lib.rs",
                    "rangeStart":100,
                    "rangeEnd":300,
                    "startLine":10,
                    "endLine":30,
                    "directReferencesTruncated":false,
                    "directReferences":[
                        {
                            "kind":"CALL_PATH",
                            "pathSegments":["row"],
                            "terminalName":"row",
                            "rangeStart":120,
                            "rangeEnd":125,
                            "startLine":12,
                            "endLine":12,
                            "resolution":"SYNTAX_UNRESOLVED"
                        },
                        {
                            "kind":"CALL_PATH",
                            "pathSegments":["super","aggregate_completeness"],
                            "terminalName":"aggregate_completeness",
                            "rangeStart":150,
                            "rangeEnd":180,
                            "startLine":16,
                            "endLine":16,
                            "resolution":"SYNTAX_UNRESOLVED"
                        },
                        {
                            "kind":"CALL_PATH",
                            "pathSegments":["member_context"],
                            "terminalName":"member_context",
                            "rangeStart":190,
                            "rangeEnd":204,
                            "startLine":18,
                            "endLine":18,
                            "resolution":"SYNTAX_UNRESOLVED"
                        },
                        {
                            "kind":"CALL_PATH",
                            "pathSegments":["unmatched_helper"],
                            "terminalName":"unmatched_helper",
                            "rangeStart":210,
                            "rangeEnd":226,
                            "startLine":20,
                            "endLine":20,
                            "resolution":"SYNTAX_UNRESOLVED"
                        },
                        {
                            "kind":"CALL_PATH",
                            "pathSegments":["member_global"],
                            "terminalName":"member_global",
                            "rangeStart":230,
                            "rangeEnd":243,
                            "startLine":22,
                            "endLine":22,
                            "resolution":"SYNTAX_UNRESOLVED"
                        }
                    ]
                }
            }],
            "sources":[],
            "completeness":{},
            "truncated":false
        });
        let mut authority = context("context:reference-parent", None, "sha256:parent", retained);
        authority.terms = vec!["completeness".into(), "member".into(), "unmatched".into()];
        let candidate_id = candidate_handle("cargo:demo", "fact:test").unwrap();
        let (selected, source_references_truncated, eligible_count) =
            select_direct_references(&authority, &candidate_id).unwrap();
        assert!(!source_references_truncated);
        assert_eq!(eligible_count, 4);
        assert_eq!(selected.len(), 3);
        assert_eq!(selected[0].terminal_name(), "aggregate_completeness");
        assert_eq!(selected[0].matched_terms, vec!["completeness"]);
        assert_eq!(
            selected[0].observations[0].path_segments,
            vec!["super", "aggregate_completeness"]
        );
        assert_eq!(selected[1].terminal_name(), "member_context");
        assert_eq!(selected[2].terminal_name(), "member_global");

        authority.projection["matches"][0]["payload"]["directReferences"][1]["terminalName"] =
            json!("different");
        authority.evidence["context"] = authority.projection.clone();
        assert_eq!(
            select_direct_references(&authority, &candidate_id)
                .unwrap_err()
                .code,
            ErrorCode::InvalidInput
        );
    }

    #[test]
    fn explicit_reference_follow_binds_canonical_paths_and_remains_unresolved() {
        let parent_retained = json!({
            "matches":[{
                "compilation":"cargo:demo",
                "factKey":"fact:caller",
                "domainUri":"analysis:rust-syntax-facts",
                "payloadRef":{"digest":"sha256:caller"},
                "payload":{
                    "schema":"codeclew-rust-syntax-fact/1.2",
                    "kind":"declaration",
                    "name":"caller",
                    "file":"src/lib.rs",
                    "rangeStart":100,
                    "rangeEnd":300,
                    "startLine":10,
                    "endLine":30,
                    "directReferencesTruncated":false,
                    "directReferences":[
                        {"kind":"CALL_PATH","pathSegments":["canonical","bytes"],"terminalName":"bytes","rangeStart":120,"rangeEnd":136,"startLine":12,"endLine":12,"resolution":"SYNTAX_UNRESOLVED"},
                        {"kind":"CALL_PATH","pathSegments":["bytes"],"terminalName":"bytes","rangeStart":137,"rangeEnd":142,"startLine":13,"endLine":13,"resolution":"SYNTAX_UNRESOLVED"},
                        {"kind":"CALL_PATH","pathSegments":["canonical","hash"],"terminalName":"hash","rangeStart":150,"rangeEnd":165,"startLine":15,"endLine":15,"resolution":"SYNTAX_UNRESOLVED"},
                        {"kind":"CALL_PATH","pathSegments":["aaa"],"terminalName":"aaa","rangeStart":170,"rangeEnd":174,"startLine":17,"endLine":17,"resolution":"SYNTAX_UNRESOLVED"},
                        {"kind":"CALL_PATH","pathSegments":["bbb"],"terminalName":"bbb","rangeStart":180,"rangeEnd":184,"startLine":18,"endLine":18,"resolution":"SYNTAX_UNRESOLVED"},
                        {"kind":"CALL_PATH","pathSegments":["ccc"],"terminalName":"ccc","rangeStart":190,"rangeEnd":194,"startLine":19,"endLine":19,"resolution":"SYNTAX_UNRESOLVED"}
                    ]
                }
            }],
            "sources":[{"fileId":"src/lib.rs","contentRef":{"digest":"sha256:source"},"windows":[{"startLine":10,"endLine":30,"text":"fn caller() { canonical::bytes(); canonical::hash(); }"}]}],
            "completeness":{},
            "truncated":false
        });
        let parent = context("context:parent", None, "sha256:parent", parent_retained);
        let candidate_id = candidate_handle("cargo:demo", "fact:caller").unwrap();
        let candidate = detail(&parent, std::slice::from_ref(&candidate_id), true, &[]).unwrap();
        assert_eq!(candidate["schema"], NAV_DETAIL_SCHEMA);
        assert_eq!(candidate["schema"], "codeclew-navigation-detail/1.1");
        assert_eq!(
            candidate["candidates"][0]["referenceChoices"]["status"],
            "SUPPORTED"
        );
        assert_eq!(
            candidate["candidates"][0]["referenceChoices"]["choices"][0]["paths"][0],
            "canonical::bytes"
        );
        assert_eq!(
            candidate["candidates"][0]["referenceChoices"]["choices"][1]["paths"][0],
            "canonical::hash"
        );
        assert_eq!(
            candidate["candidates"][0]["referenceChoices"]["truncated"],
            true
        );
        assert!(
            candidate["candidates"][0]["referenceChoices"]["nextAction"]
                .as_str()
                .unwrap()
                .contains("--reference")
        );
        assert_eq!(
            candidate["candidates"][0]["referenceChoices"]["followAction"],
            json!({
                "kind":"RETAINED_REFERENCE_FOLLOW",
                "candidateId":candidate_id,
                "maxReferences":3,
                "onePathPerTerminal":true,
                "includeSource":true,
                "includeFacet":false,
                "requiresNewestContext":true,
                "choiceAuthority":"RETAINED_DIRECT_REFERENCE_FACT",
                "resultSelectionAuthority":"USER_SELECTED_RETAINED_REFERENCE",
                "targetResolution":"UNRESOLVED",
                "semanticRelation":"UNKNOWN",
            })
        );
        let mut foreign = parent.clone();
        foreign.evidence["context"]["matches"][0]["payload"]["schema"] =
            json!("codeclew-typescript-syntax-fact/1.0");
        let foreign_detail =
            detail(&foreign, std::slice::from_ref(&candidate_id), true, &[]).unwrap();
        assert_eq!(
            foreign_detail["candidates"][0]["referenceChoices"]["status"],
            "UNSUPPORTED"
        );
        assert_eq!(
            select_explicit_direct_references(&foreign, &candidate_id, &["bytes".into()])
                .unwrap_err()
                .code,
            ErrorCode::UnsupportedProjectConfiguration
        );
        foreign.evidence["context"]["matches"][0]["payload"]["schema"] =
            json!("codeclew-rust-syntax-fact/1.0");
        assert_eq!(
            detail(&foreign, std::slice::from_ref(&candidate_id), true, &[]).unwrap()["candidates"]
                [0]["referenceChoices"]["status"],
            "UNSUPPORTED"
        );

        let selections = select_explicit_direct_references(
            &parent,
            &candidate_id,
            &["canonical::bytes".into(), "hash".into()],
        )
        .unwrap();
        assert_eq!(selections[0].terminal_name(), "bytes");
        assert_eq!(
            selections[0].observations[0].path_segments,
            vec!["canonical", "bytes"]
        );
        assert_eq!(selections[1].terminal_name(), "hash");
        assert_eq!(
            select_explicit_direct_references(&parent, &candidate_id, &["missing".into()])
                .unwrap_err()
                .code,
            ErrorCode::SymbolNotFound
        );
        assert_eq!(
            select_explicit_direct_references(
                &parent,
                &candidate_id,
                &[
                    "bytes".into(),
                    "hash".into(),
                    "other".into(),
                    "fourth".into(),
                ],
            )
            .unwrap_err()
            .code,
            ErrorCode::InvalidInput
        );
        let mut truncated_parent = parent.clone();
        truncated_parent.evidence["context"]["matches"][0]["payload"]["directReferencesTruncated"] =
            json!(true);
        assert_eq!(
            select_explicit_direct_references(
                &truncated_parent,
                &candidate_id,
                &["missing".into()]
            )
            .unwrap_err()
            .code,
            ErrorCode::IncompleteSemanticAnalysis
        );

        let child_retained = json!({
            "matches":[
                {"compilation":"cargo:demo","factKey":"fact:bytes","payload":{"kind":"declaration","name":"bytes","file":"src/canonical.rs","startLine":1,"endLine":1}},
                {"compilation":"cargo:demo","factKey":"fact:hash","payload":{"kind":"declaration","name":"hash","file":"src/canonical.rs","startLine":2,"endLine":2}}
            ],
            "sources":[{"fileId":"src/canonical.rs","contentRef":{"digest":"sha256:canonical"},"windows":[{"startLine":1,"endLine":2,"text":"fn bytes() {}\nfn hash() {}"}]}],
            "completeness":{},
            "truncated":false
        });
        let child = context(
            "context:child",
            Some("context:parent"),
            "sha256:child",
            child_retained,
        );
        let followed = explicit_direct_reference_detail(&child, &selections).unwrap();
        assert_eq!(
            followed["selectionAuthority"],
            "USER_SELECTED_RETAINED_REFERENCE"
        );
        assert_eq!(followed["targetResolution"], "UNRESOLVED");
        assert_eq!(followed["semanticRelation"], "UNKNOWN");
        assert_eq!(followed["references"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn direct_reference_selection_abstains_before_requiring_adapter_specific_coordinates() {
        let retained = json!({
            "matches":[{
                "compilation":"tsconfig:tsconfig.json",
                "factKey":"typescript:declaration:api-error",
                "payload":{
                    "kind":"DECLARATION",
                    "name":"ApiError",
                    "file":"src/client.ts",
                    "start":100,
                    "end":180
                }
            }],
            "sources":[],
            "completeness":{},
            "truncated":false
        });
        let mut authority = context(
            "context:typescript-declaration",
            None,
            "sha256:typescript-declaration",
            retained,
        );
        let candidate_id =
            candidate_handle("tsconfig:tsconfig.json", "typescript:declaration:api-error").unwrap();

        assert_eq!(
            select_direct_references(&authority, &candidate_id).unwrap(),
            (Vec::new(), false, 0)
        );

        authority.projection["matches"][0]["payload"]["directReferencesTruncated"] = json!(false);
        authority.evidence["context"] = authority.projection.clone();
        assert_eq!(
            select_direct_references(&authority, &candidate_id)
                .unwrap_err()
                .code,
            ErrorCode::InvalidInput
        );

        authority.projection["matches"][0]["payload"]["directReferences"] = json!([]);
        authority.evidence["context"] = authority.projection.clone();
        assert_eq!(
            select_direct_references(&authority, &candidate_id)
                .unwrap_err()
                .code,
            ErrorCode::InvalidInput
        );
    }

    #[test]
    fn direct_reference_detail_keeps_all_bounded_namesakes_and_same_file_is_only_ordering() {
        let retained = json!({
            "matches":[
                {
                    "compilation":"cargo:demo",
                    "factKey":"fact:other",
                    "payload":{
                        "kind":"declaration",
                        "name":"aggregate_completeness",
                        "declarationKind":"function",
                        "symbolIdentity":"symbol:other",
                        "file":"src/other.rs",
                        "rangeStart":10,
                        "rangeEnd":30,
                        "startLine":2,
                        "endLine":4
                    }
                },
                {
                    "compilation":"cargo:demo",
                    "factKey":"fact:local",
                    "payload":{
                        "kind":"declaration",
                        "name":"aggregate_completeness",
                        "declarationKind":"function",
                        "symbolIdentity":"symbol:local",
                        "file":"src/lib.rs",
                        "rangeStart":100,
                        "rangeEnd":140,
                        "startLine":20,
                        "endLine":23
                    }
                }
            ],
            "sources":[
                {
                    "fileId":"src/other.rs",
                    "contentRef":{"digest":"sha256:other"},
                    "windows":[{"startLine":2,"endLine":4,"text":"fn aggregate_completeness() {\n    other();\n}"}]
                },
                {
                    "fileId":"src/lib.rs",
                    "contentRef":{"digest":"sha256:local"},
                    "windows":[{"startLine":20,"endLine":23,"text":"fn aggregate_completeness() {\n    local();\n}"}]
                }
            ],
            "completeness":{"certainty":"UNSURE"},
            "truncated":false
        });
        let authority = context(
            "context:reference-child",
            Some("context:reference-parent"),
            "sha256:child",
            retained,
        );
        let selection = DirectReferenceSelection {
            terminal_name: "aggregate_completeness".into(),
            source_file: "src/lib.rs".into(),
            matched_terms: vec!["completeness".into()],
            observations: vec![DirectReferenceObservation {
                path_segments: vec!["super".into(), "aggregate_completeness".into()],
                range_start: 150,
                range_end: 180,
                start_line: 16,
                end_line: 16,
            }],
            source_references_truncated: false,
        };
        let detail = direct_reference_detail(&authority, &[selection], 2).unwrap();
        assert_eq!(detail["semanticRelation"], "UNKNOWN");
        assert_eq!(detail["targetResolution"], "UNRESOLVED");
        assert_eq!(detail["selectionTruncated"], true);
        assert_eq!(detail["referenceTermCount"]["eligible"], 2);
        assert_eq!(detail["referenceTermCount"]["omitted"], 1);
        assert_eq!(detail["truncated"], true);
        assert_eq!(
            detail["references"][0]["nameMatches"]["status"],
            "AMBIGUOUS_RETAINED_NAME"
        );
        assert_eq!(
            detail["references"][0]["nameMatches"]["ordering"],
            "SAME_FILE_FIRST_PRESENTATION_ONLY"
        );
        assert_eq!(
            detail["references"][0]["nameMatches"]["candidates"][0]["sameFileAsObservation"],
            true
        );
        assert_eq!(
            detail["references"][0]["nameMatches"]["candidates"][0]["source"]["status"],
            "SUPPORTED"
        );
        assert_eq!(
            detail["references"][0]["nameMatches"]["candidates"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn direct_reference_detail_batches_three_selected_terms_under_one_source_budget() {
        let declaration = |fact_key: &str, name: &str, file: &str, line: u64| {
            json!({
                "compilation":"cargo:demo",
                "factKey":fact_key,
                "payload":{
                    "kind":"declaration",
                    "name":name,
                    "declarationKind":"function",
                    "symbolIdentity":format!("symbol:{name}"),
                    "file":file,
                    "rangeStart":line * 10,
                    "rangeEnd":line * 10 + 5,
                    "startLine":1,
                    "endLine":1
                }
            })
        };
        let retained = json!({
            "matches":[
                declaration("fact:alpha", "alpha", "src/alpha.rs", 1),
                declaration("fact:beta", "beta", "src/beta.rs", 2),
                declaration("fact:gamma", "gamma", "src/gamma.rs", 3),
            ],
            "sources":[
                {
                    "fileId":"src/alpha.rs",
                    "contentRef":{"digest":"sha256:alpha"},
                    "windows":[{"startLine":1,"endLine":1,"text":"fn alpha() {}"}]
                },
                {
                    "fileId":"src/beta.rs",
                    "contentRef":{"digest":"sha256:beta"},
                    "windows":[{"startLine":1,"endLine":1,"text":"fn beta() {}"}]
                },
                {
                    "fileId":"src/gamma.rs",
                    "contentRef":{"digest":"sha256:gamma"},
                    "windows":[{"startLine":1,"endLine":1,"text":"fn gamma() {}"}]
                }
            ],
            "completeness":{"certainty":"UNSURE"},
            "truncated":false
        });
        let authority = context(
            "context:three-reference-child",
            Some("context:three-reference-parent"),
            "sha256:three-reference-child",
            retained,
        );
        let selections = ["alpha", "beta", "gamma"]
            .into_iter()
            .enumerate()
            .map(|(index, terminal_name)| DirectReferenceSelection {
                terminal_name: terminal_name.into(),
                source_file: "src/caller.rs".into(),
                matched_terms: vec![terminal_name.into()],
                observations: vec![DirectReferenceObservation {
                    path_segments: vec![terminal_name.into()],
                    range_start: 100 + index as u64 * 10,
                    range_end: 105 + index as u64 * 10,
                    start_line: 10 + index as u64,
                    end_line: 10 + index as u64,
                }],
                source_references_truncated: false,
            })
            .collect::<Vec<_>>();

        let detail = direct_reference_detail(&authority, &selections, 4).unwrap();
        assert_eq!(detail["sessionId"], "session:test");
        assert_eq!(detail["contextId"], "context:three-reference-child");
        assert_eq!(detail["evidenceDigest"], "sha256:three-reference-child");
        assert_eq!(detail["references"].as_array().unwrap().len(), 3);
        assert_eq!(detail["referenceTermCount"]["returned"], 3);
        assert_eq!(detail["referenceTermCount"]["eligible"], 4);
        assert_eq!(detail["referenceTermCount"]["omitted"], 1);
        assert_eq!(detail["selectionTruncated"], true);
        assert!(detail["sourceBytes"].as_u64().unwrap() > 0);
        assert!(
            detail["sourceBytes"].as_u64().unwrap()
                <= u64::try_from(MAX_REFERENCE_FOLLOW_SOURCE_BYTES).unwrap()
        );
        assert!(
            detail["references"].as_array().unwrap().iter().all(|row| {
                row["nameMatches"]["candidates"][0]["source"]["status"] == "SUPPORTED"
            })
        );
        assert_eq!(
            detail["references"]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| row["observed"]["terminalName"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["alpha", "beta", "gamma"]
        );
    }

    #[test]
    fn direct_reference_detail_omits_later_source_after_the_shared_budget() {
        let declaration = |fact_key: &str, name: &str, file: &str| {
            json!({
                "compilation":"cargo:demo",
                "factKey":fact_key,
                "payload":{
                    "kind":"declaration",
                    "name":name,
                    "declarationKind":"function",
                    "symbolIdentity":format!("symbol:{name}"),
                    "file":file,
                    "rangeStart":10,
                    "rangeEnd":20,
                    "startLine":1,
                    "endLine":2
                }
            })
        };
        let retained = json!({
            "matches":[
                declaration("fact:alpha", "alpha", "src/alpha.rs"),
                declaration("fact:beta", "beta", "src/beta.rs"),
                declaration("fact:gamma", "gamma", "src/gamma.rs"),
            ],
            "sources":[
                {
                    "fileId":"src/alpha.rs",
                    "contentRef":{"digest":"sha256:alpha"},
                    "windows":[{
                        "startLine":1,
                        "endLine":2,
                        "text":format!("fn alpha() {{}}\n{}", "ALPHA".repeat(1_800))
                    }]
                },
                {
                    "fileId":"src/beta.rs",
                    "contentRef":{"digest":"sha256:beta"},
                    "windows":[{
                        "startLine":1,
                        "endLine":2,
                        "text":format!("fn beta() {{}}\n{}", "SECRET_BETA_PAYLOAD".repeat(500))
                    }]
                },
                {
                    "fileId":"src/gamma.rs",
                    "contentRef":{"digest":"sha256:gamma"},
                    "windows":[{
                        "startLine":1,
                        "endLine":2,
                        "text":format!("fn gamma() {{}}\n{}", "SECRET_GAMMA_PAYLOAD".repeat(500))
                    }]
                }
            ],
            "completeness":{"certainty":"UNSURE"},
            "truncated":false
        });
        let authority = context(
            "context:reference-budget-child",
            Some("context:reference-budget-parent"),
            "sha256:reference-budget-child",
            retained,
        );
        let selections = ["alpha", "beta", "gamma"]
            .into_iter()
            .map(|terminal_name| DirectReferenceSelection {
                terminal_name: terminal_name.into(),
                source_file: "src/caller.rs".into(),
                matched_terms: vec![terminal_name.into()],
                observations: vec![DirectReferenceObservation {
                    path_segments: vec![terminal_name.into()],
                    range_start: 100,
                    range_end: 105,
                    start_line: 10,
                    end_line: 10,
                }],
                source_references_truncated: false,
            })
            .collect::<Vec<_>>();

        let detail = direct_reference_detail(&authority, &selections, 3).unwrap();
        assert_eq!(
            detail["references"][0]["nameMatches"]["candidates"][0]["source"]["status"],
            "SUPPORTED"
        );
        for index in [1, 2] {
            assert_eq!(
                detail["references"][index]["nameMatches"]["candidates"][0]["source"]["status"],
                "UNAVAILABLE"
            );
            assert_eq!(
                detail["references"][index]["nameMatches"]["candidates"][0]["source"]["reason"],
                "OMITTED_BUDGET"
            );
        }
        assert_eq!(detail["truncated"], true);
        assert!(
            detail["sourceBytes"].as_u64().unwrap()
                <= u64::try_from(MAX_REFERENCE_FOLLOW_SOURCE_BYTES).unwrap()
        );
        let public = serde_json::to_string(&detail).unwrap();
        assert!(!public.contains("SECRET_BETA_PAYLOAD"));
        assert!(!public.contains("SECRET_GAMMA_PAYLOAD"));
    }

    #[test]
    fn direct_reference_detail_rejects_empty_or_over_limit_selection_sets() {
        let authority = context(
            "context:reference-bounds-child",
            Some("context:reference-bounds-parent"),
            "sha256:reference-bounds-child",
            json!({
                "matches":[],
                "sources":[],
                "completeness":{"certainty":"UNSURE"},
                "truncated":false
            }),
        );
        assert_eq!(
            direct_reference_detail(&authority, &[], 0)
                .unwrap_err()
                .code,
            ErrorCode::InvalidInput
        );
        let over_limit = ["alpha", "beta", "gamma", "delta"]
            .into_iter()
            .map(|terminal_name| DirectReferenceSelection {
                terminal_name: terminal_name.into(),
                source_file: "src/caller.rs".into(),
                matched_terms: vec![terminal_name.into()],
                observations: Vec::new(),
                source_references_truncated: false,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            direct_reference_detail(&authority, &over_limit, 4)
                .unwrap_err()
                .code,
            ErrorCode::InvalidInput
        );
    }

    #[test]
    fn detail_batches_independent_exact_candidates() {
        let retained = json!({
            "matches":[
                {
                    "compilation":"cargo:demo",
                    "factKey":"fact:first",
                    "payload":{
                        "kind":"declaration",
                        "name":"First",
                        "symbolIdentity":"symbol:First",
                        "file":"src/first.rs",
                        "startLine":2,
                        "endLine":2
                    }
                },
                {
                    "compilation":"cargo:demo",
                    "factKey":"fact:second",
                    "payload":{
                        "kind":"declaration",
                        "name":"Second",
                        "symbolIdentity":"symbol:Second",
                        "file":"src/second.rs",
                        "startLine":4,
                        "endLine":4
                    }
                }
            ],
            "sources":[
                {
                    "fileId":"src/first.rs",
                    "contentRef":{"digest":"sha256:first"},
                    "windows":[{"startLine":2,"endLine":2,"text":"fn First() {}"}]
                },
                {
                    "fileId":"src/second.rs",
                    "contentRef":{"digest":"sha256:second"},
                    "windows":[{"startLine":4,"endLine":4,"text":"fn Second() {}"}]
                }
            ],
            "completeness":{},
            "truncated":false
        });
        let authority = context("context:batch", None, "sha256:evidence", retained);
        let cards = query(&authority, &[]).unwrap();
        let candidate_ids = cards["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .map(|card| card["candidateId"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        let selected = detail(&authority, &candidate_ids, true, &[]).unwrap();
        assert_eq!(selected["candidates"].as_array().unwrap().len(), 2);
        assert_eq!(
            selected["candidates"][0]["fact"]["payload"]["name"],
            "First"
        );
        assert_eq!(
            selected["candidates"][0]["source"]["contentRef"]["digest"],
            "sha256:first"
        );
        assert_eq!(
            selected["candidates"][1]["fact"]["payload"]["name"],
            "Second"
        );
        assert_eq!(
            selected["candidates"][1]["source"]["contentRef"]["digest"],
            "sha256:second"
        );
        assert_eq!(
            detail(
                &authority,
                &[candidate_ids[0].clone(), candidate_ids[0].clone()],
                true,
                &[],
            )
            .unwrap_err()
            .code,
            ErrorCode::InvalidInput
        );
        assert_eq!(
            detail(
                &authority,
                &[
                    "c:0000000000000000".into(),
                    "c:1111111111111111".into(),
                    "c:2222222222222222".into(),
                    "c:3333333333333333".into(),
                ],
                true,
                &[],
            )
            .unwrap_err()
            .code,
            ErrorCode::InvalidInput
        );
    }

    #[test]
    fn exact_file_term_selects_one_declaration_and_refuses_ambiguity() {
        let retained = json!({
            "matches":[
                {
                    "compilation":"cargo:demo",
                    "factKey":"fact:a",
                    "payload":{
                        "kind":"declaration",
                        "name":"Target",
                        "symbolIdentity":"symbol:a:Target",
                        "file":"src/a.rs",
                        "startLine":1,
                        "endLine":1
                    }
                },
                {
                    "compilation":"cargo:demo",
                    "factKey":"fact:b",
                    "payload":{
                        "kind":"declaration",
                        "name":"Target",
                        "symbolIdentity":"symbol:b:Target",
                        "file":"src/b.rs",
                        "rangeStart":20,
                        "rangeEnd":100,
                        "startLine":2,
                        "endLine":2,
                        "matchArmCases":{
                            "authority":"EXACT_SNAPSHOT_TEXT_SYNTAX",
                            "coverage":"VISIBLE_PARSED_MATCH_ARMS_COMPLETE",
                            "cases":[
                                {"caseId":"rust-case:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","kind":"MATCH_ARM","declarationIdentity":"symbol:b:Target","groupStart":30,"pattern":{"start":31,"end":32,"text":"A"},"guard":null,"body":{"start":40,"end":41,"text":"a"},"authority":"EXACT_SNAPSHOT_TEXT_SYNTAX"},
                                {"caseId":"rust-case:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","kind":"MATCH_ARM","declarationIdentity":"symbol:b:Target","groupStart":30,"pattern":{"start":42,"end":43,"text":"B"},"guard":null,"body":{"start":50,"end":51,"text":"b"},"authority":"EXACT_SNAPSHOT_TEXT_SYNTAX"},
                                {"caseId":"rust-case:sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc","kind":"MATCH_ARM","declarationIdentity":"symbol:b:Target","groupStart":30,"pattern":{"start":52,"end":53,"text":"C"},"guard":null,"body":{"start":60,"end":61,"text":"c"},"authority":"EXACT_SNAPSHOT_TEXT_SYNTAX"}
                            ]
                        }
                    }
                }
            ],
            "sources":[
                {
                    "fileId":"src/a.rs",
                    "contentRef":{"digest":"sha256:a"},
                    "windows":[{"startLine":1,"endLine":1,"text":"fn Target() {}"}]
                },
                {
                    "fileId":"src/b.rs",
                    "contentRef":{"digest":"sha256:b"},
                    "windows":[{"startLine":2,"endLine":2,"text":"fn Target() {}"}]
                }
            ],
            "completeness":{},
            "truncated":false
        });
        let authority = context("context:exact", None, "sha256:evidence", retained);
        let selected = detail_by_exact_file_term(&authority, "src/b.rs", "Target", true).unwrap();
        assert_eq!(
            selected["candidates"][0]["candidateKey"]["factKey"],
            "fact:b"
        );
        assert_eq!(
            selected["candidates"][0]["source"]["contentRef"]["digest"],
            "sha256:b"
        );
        assert_eq!(
            selected["candidates"][0]["fact"]["payload"]["matchArmCases"]["cases"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        assert_eq!(
            detail_by_exact_file_term(&authority, "../src/b.rs", "Target", true)
                .unwrap_err()
                .code,
            ErrorCode::InvalidInput
        );
        assert_eq!(
            detail_by_exact_file_term(&authority, "src//b.rs", "Target", true)
                .unwrap_err()
                .code,
            ErrorCode::InvalidInput
        );

        let mut ambiguous = authority.clone();
        ambiguous.evidence["context"]["matches"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "compilation":"cargo:other",
                "factKey":"fact:b-overload",
                "payload":{
                    "kind":"declaration",
                    "name":"Target",
                    "symbolIdentity":"symbol:b:Target-overload",
                    "file":"src/b.rs",
                    "startLine":2,
                    "endLine":2
                }
            }));
        assert_eq!(
            detail_by_exact_file_term(&ambiguous, "src/b.rs", "Target", true)
                .unwrap_err()
                .code,
            ErrorCode::AmbiguousSymbol
        );
    }

    #[test]
    fn exact_file_terms_select_three_declarations_in_requested_order() {
        let retained = json!({
            "matches":[
                {
                    "compilation":"cargo:demo",
                    "factKey":"fact:one",
                    "payload":{"kind":"declaration","name":"CONST1","file":"src/cas.rs","startLine":1,"endLine":1}
                },
                {
                    "compilation":"cargo:demo",
                    "factKey":"fact:two",
                    "payload":{"kind":"declaration","name":"CONST2","file":"src/cas.rs","startLine":2,"endLine":2}
                },
                {
                    "compilation":"cargo:demo",
                    "factKey":"fact:three",
                    "payload":{"kind":"declaration","name":"CONST3","file":"src/cas.rs","startLine":3,"endLine":3}
                }
            ],
            "sources":[{
                "fileId":"src/cas.rs",
                "contentRef":{"digest":"sha256:cas"},
                "windows":[{"startLine":1,"endLine":3,"text":"const CONST1;\nconst CONST2;\nconst CONST3;"}]
            }],
            "completeness":{},
            "truncated":false
        });
        let authority = context("context:exact-batch", None, "sha256:evidence", retained);
        let terms = vec!["CONST3".into(), "CONST1".into(), "CONST2".into()];
        let selected = detail_by_exact_file_terms(&authority, "src/cas.rs", &terms, true).unwrap();
        assert_eq!(selected["candidates"].as_array().unwrap().len(), 3);
        assert_eq!(
            selected["candidates"][0]["candidateKey"]["factKey"],
            "fact:three"
        );
        assert_eq!(
            selected["candidates"][1]["candidateKey"]["factKey"],
            "fact:one"
        );
        assert_eq!(
            selected["candidates"][2]["candidateKey"]["factKey"],
            "fact:two"
        );
        assert!(
            selected["candidates"]
                .as_array()
                .unwrap()
                .iter()
                .all(|candidate| { candidate["source"]["status"] == "SUPPORTED" })
        );
        assert_eq!(
            detail_by_exact_file_terms(
                &authority,
                "src/cas.rs",
                &["CONST1".into(), "CONST1".into()],
                true,
            )
            .unwrap_err()
            .code,
            ErrorCode::InvalidInput
        );
        assert_eq!(
            detail_by_exact_file_terms(
                &authority,
                "src/cas.rs",
                &["CONST1".into(), "MISSING".into()],
                true,
            )
            .unwrap_err()
            .code,
            ErrorCode::SymbolNotFound
        );
        assert_eq!(
            detail_by_exact_file_terms(
                &authority,
                "src/cas.rs",
                &[
                    "CONST1".into(),
                    "CONST2".into(),
                    "CONST3".into(),
                    "CONST4".into(),
                ],
                true,
            )
            .unwrap_err()
            .code,
            ErrorCode::InvalidInput
        );

        let mut cross_file = authority.clone();
        cross_file.evidence["context"]["matches"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "compilation":"cargo:demo",
                "factKey":"fact:other",
                "payload":{"kind":"declaration","name":"OTHER","file":"src/other.rs","startLine":1,"endLine":1}
            }));
        assert_eq!(
            detail_by_exact_file_terms(
                &cross_file,
                "src/cas.rs",
                &["CONST1".into(), "OTHER".into()],
                true,
            )
            .unwrap_err()
            .code,
            ErrorCode::SymbolNotFound
        );
    }

    #[test]
    fn decision_preview_bounds_a_valid_minified_source_line() {
        let long_line = format!("fn Target() {{ {} }}", "x".repeat(32 * 1024));
        let retained = json!({
            "matches":[{
                "compilation":"cargo:demo",
                "factKey":"fact:target",
                "payload":{
                    "kind":"declaration",
                    "name":"Target",
                    "symbolIdentity":"symbol:Target",
                    "file":"src/lib.rs",
                    "startLine":1,
                    "endLine":1
                }
            }],
            "sources":[{
                "fileId":"src/lib.rs",
                "contentRef":{"digest":"sha256:source"},
                "startLine":1,
                "endLine":1,
                "text":long_line,
                "windows":[{"startLine":1,"endLine":1,"text":long_line}],
                "completeFile":true
            }],
            "completeness":{},
            "truncated":false
        });
        let result = assemble(
            "session:test",
            "context:test",
            "sha256:evidence",
            NAV_QUERY_INTENT,
            &["Target".into()],
            &retained,
            false,
            &[],
        )
        .unwrap();
        assert!(
            result["candidates"][0]["preview"]["text"]
                .as_str()
                .unwrap()
                .len()
                <= 512
        );
        assert_eq!(result["candidates"][0]["preview"]["truncated"], true);
        validate_stdout(&result).unwrap();
    }

    #[test]
    fn decision_card_bounds_large_payload_strings_without_touching_detail() {
        let retained = json!({
            "matches":[{
                "compilation":"cargo:demo",
                "factKey":"fact:large",
                "payload":{
                    "kind":"declaration",
                    "name":"N".repeat(20 * 1024),
                    "declarationKind":"K".repeat(20 * 1024),
                    "symbolIdentity":"S".repeat(20 * 1024),
                    "file":"src/lib.rs",
                    "startLine":1,
                    "endLine":1
                }
            }],
            "sources":[],
            "completeness":{},
            "truncated":false
        });
        let result = assemble(
            "session:test",
            "context:test",
            "sha256:evidence",
            NAV_QUERY_INTENT,
            &["large".into()],
            &retained,
            false,
            &[],
        )
        .unwrap();
        let card = &result["candidates"][0];
        for field in ["displayName", "declarationKind", "symbolIdentity"] {
            assert!(card[field].as_str().unwrap().len() <= 512);
        }
        assert_eq!(card["displayNameTruncated"], true);
        assert_eq!(card["declarationKindTruncated"], true);
        assert_eq!(card["symbolIdentityTruncated"], true);
        validate_stdout(&result).unwrap();
    }

    #[test]
    fn top_three_cap_marks_truncation_and_bounds_facets() {
        let mut matches = (0..4)
            .map(|index| {
                let name = char::from(b'A' + index as u8).to_string();
                json!({
                    "compilation":"cargo:demo",
                    "factKey":format!("fact:{name}"),
                    "payload":{
                        "kind":"declaration",
                        "name":name,
                        "symbolIdentity":format!("symbol:{name}"),
                        "file":"src/lib.rs",
                        "startLine":index + 1,
                        "endLine":index + 1,
                    }
                })
            })
            .collect::<Vec<_>>();
        matches.extend([
            json!({
                "compilation":"cargo:demo","factKey":"edge:A",
                "payload":{"kind":"relation","relationKind":"CALL","sourceIdentity":"symbol:CallerA","targetIdentity":"symbol:A"}
            }),
            json!({
                "compilation":"cargo:demo","factKey":"edge:D",
                "payload":{"kind":"relation","relationKind":"CALL","sourceIdentity":"symbol:CallerD","targetIdentity":"symbol:D"}
            }),
        ]);
        let retained = json!({
            "matches":matches,
            "sources":[],
            "completeness":{"certainty":"VERIFIED"},
            "truncated":false
        });
        let result = assemble(
            "session:test",
            "context:test",
            "sha256:evidence",
            NAV_QUERY_INTENT,
            &["target".into()],
            &retained,
            false,
            &[NavigationFacet::Callers],
        )
        .unwrap();
        assert_eq!(result["candidateCount"]["returned"], 3);
        assert_eq!(result["candidateCount"]["omitted"], 1);
        assert_eq!(result["truncated"], true);
        let edges = result["facets"]["callers"]["edges"].as_array().unwrap();
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0]["factKey"], "edge:A");
    }

    #[test]
    fn decision_cards_rank_exact_retained_source_coverage_before_generic_names() {
        let declaration = |name: &str, line: u64| {
            json!({
                "compilation":"cargo:demo",
                "factKey":format!("fact:{name}"),
                "payload":{
                    "kind":"declaration",
                    "name":name,
                    "symbolIdentity":format!("symbol:{name}"),
                    "file":"src/lib.rs",
                    "startLine":line,
                    "endLine":line,
                }
            })
        };
        let retained = json!({
            "matches":[
                declaration("AlphaBeta", 1),
                declaration("Alpha", 3),
                declaration("Work", 5),
                declaration("Gamma", 7),
            ],
            "sources":[{
                "fileId":"src/lib.rs",
                "contentRef":{"digest":"sha256:source"},
                "windows":[
                    {"startLine":1,"endLine":1,"text":"fn AlphaBeta() {}"},
                    {"startLine":3,"endLine":3,"text":"fn Alpha() {}"},
                    {"startLine":5,"endLine":5,"text":"fn Work() { alpha(beta, gamma); }"},
                    {"startLine":7,"endLine":7,"text":"fn Gamma() {}"}
                ]
            }],
            "completeness":{},
            "truncated":false,
        });
        let result = assemble(
            "session:test",
            "context:test",
            "sha256:evidence",
            NAV_QUERY_INTENT,
            &["alpha".into(), "beta".into(), "gamma".into()],
            &retained,
            false,
            &[],
        )
        .unwrap();
        assert_eq!(result["candidates"][0]["displayName"], "Work");
        assert_eq!(result["candidates"][1]["displayName"], "AlphaBeta");
        assert_eq!(result["candidateCount"]["omitted"], 1);
    }

    #[test]
    fn decision_cards_prefer_the_exact_file_that_covers_more_query_terms() {
        let retained = json!({
            "matches":[
                {
                    "compilation":"tsconfig:app/tsconfig.json",
                    "factKey":"fact:json-response",
                    "payload":{
                        "kind":"declaration",
                        "name":"jsonResponse",
                        "symbolIdentity":"ts:test.ts#function:jsonResponse",
                        "file":"test.ts",
                        "startLine":1,
                        "endLine":4
                    }
                },
                {
                    "compilation":"tsconfig:app/tsconfig.json",
                    "factKey":"fact:api-error",
                    "payload":{
                        "kind":"declaration",
                        "name":"ApiError",
                        "symbolIdentity":"ts:client.ts#class:ApiError",
                        "file":"client.ts",
                        "startLine":1,
                        "endLine":3
                    }
                }
            ],
            "sources":[
                {
                    "fileId":"test.ts",
                    "contentRef":{"digest":"sha256:test"},
                    "windows":[{
                        "startLine":1,
                        "endLine":4,
                        "text":"function jsonResponse(payload: unknown) {\n return new Response(JSON.stringify(payload), {\n  headers: {'Content-Type':'application/json'}\n })\n}"
                    }]
                },
                {
                    "fileId":"client.ts",
                    "contentRef":{"digest":"sha256:client"},
                    "windows":[{
                        "startLine":1,
                        "endLine":9,
                        "text":"class ApiError extends Error {\n status: number\n}\nheaders.set('Content-Type', 'application/json')\nheaders.set('X-Launchpad-Source', 'ui')\nconst payload = await response.text()\nthrow new ApiError(status, payload)\nconst contentType = response.headers.get('content-type')\nreturn contentType ? response.json() : payload"
                    }]
                }
            ],
            "completeness":{},
            "truncated":false
        });
        let result = assemble(
            "session:test",
            "context:test",
            "sha256:evidence",
            NAV_QUERY_INTENT,
            &[
                "ApiError".into(),
                "Content-Type".into(),
                "X-Launchpad-Source".into(),
                "response.text".into(),
                "response.json".into(),
                "JSON.stringify".into(),
                "error".into(),
                "payload".into(),
            ],
            &retained,
            false,
            &[],
        )
        .unwrap();
        assert_eq!(result["candidates"][0]["displayName"], "ApiError");
    }

    #[test]
    fn typed_abstention_blocks_a_popular_analogue_when_explicit_terms_are_unmatched() {
        let declaration = |name: &str, file: &str, line: u64, identity: &str| {
            json!({
                "compilation":"tsconfig:app/tsconfig.json",
                "factKey":format!("fact:{name}"),
                "payload":{
                    "kind":"declaration",
                    "name":name,
                    "symbolIdentity":identity,
                    "file":file,
                    "startLine":line,
                    "endLine":line,
                }
            })
        };
        let retained = json!({
            "matches":[
                declaration(
                    "KafkaFlowExplorerPage",
                    "src/KafkaFlowExplorerPage.tsx",
                    1,
                    "symbol:KafkaFlowExplorerPage"
                ),
                declaration(
                    "DataToolsPage",
                    "src/DataToolsPage.tsx",
                    1,
                    "symbol:DataToolsPage"
                ),
                declaration("KafkaPanel", "src/KafkaPanel.tsx", 1, "symbol:KafkaPanel"),
                declaration("PollingPanel", "src/PollingPanel.tsx", 1, "symbol:PollingPanel"),
                {
                    "compilation":"tsconfig:app/tsconfig.json",
                    "factKey":"edge:data-tools-stale-poll-test",
                    "payload":{
                        "kind":"relation",
                        "relationKind":"TEST_COVERS",
                        "sourceIdentity":"test:staleOldStreamPollAfterResubscribe",
                        "targetIdentity":"symbol:DataToolsPage"
                    }
                }
            ],
            "sources":[
                {
                    "fileId":"src/KafkaFlowExplorerPage.tsx",
                    "contentRef":{"digest":"sha256:popular"},
                    "windows":[{"startLine":1,"endLine":1,"text":"function KafkaFlowExplorerPage() { kafka(); poll(); kafka(); poll(); }"}]
                },
                {
                    "fileId":"src/DataToolsPage.tsx",
                    "contentRef":{"digest":"sha256:target"},
                    "windows":[{"startLine":1,"endLine":1,"text":"function DataToolsPage() { kafka(); poll(); streamPollRequestIdRef.current += 1 }"}]
                },
                {
                    "fileId":"src/KafkaPanel.tsx",
                    "contentRef":{"digest":"sha256:kafka"},
                    "windows":[{"startLine":1,"endLine":1,"text":"function KafkaPanel() { kafka() }"}]
                },
                {
                    "fileId":"src/PollingPanel.tsx",
                    "contentRef":{"digest":"sha256:poll"},
                    "windows":[{"startLine":1,"endLine":1,"text":"function PollingPanel() { poll() }"}]
                }
            ],
            "completeness":{
                "status":"CONDITIONAL_TASK",
                "coverage":"PARTIAL",
                "certainty":"UNSURE",
                "unmatchedTerms":["subscription"]
            },
            "truncated":false
        });
        let result = assemble(
            "session:frozen-docs",
            "context:frozen-docs",
            "sha256:evidence",
            NAV_QUERY_INTENT,
            &["kafka".into(), "poll".into(), "subscription".into()],
            &retained,
            false,
            &[NavigationFacet::Tests],
        )
        .unwrap();

        assert_eq!(
            result["candidateCount"],
            json!({"returned":3,"total":4,"omitted":1})
        );
        assert_eq!(
            result["candidates"][0]["displayName"],
            "KafkaFlowExplorerPage"
        );
        assert_eq!(result["decisionAuthority"]["status"], "ABSTAIN");
        assert_eq!(
            result["decisionAuthority"]["classification"],
            "UNMATCHED_EXPLICIT_TERMS"
        );
        assert_eq!(
            result["decisionAuthority"]["refinement"]["requiredCoverage"],
            json!(["subscription"])
        );
        assert_eq!(
            result["decisionAuthority"]["refinement"]["precondition"],
            "NEW_TASK_SUPPLIED_EXACT_IDENTIFIER_NOT_ALREADY_IN_CONTEXT"
        );
        assert_eq!(
            result["decisionAuthority"]["refinement"]["repeatSameRequest"],
            false
        );
        assert_eq!(
            result["decisionAuthority"]["refinement"]["onUnsatisfied"],
            "STOP_UNRESOLVED"
        );
        assert!(
            !canonical::compact(&result["decisionAuthority"])
                .unwrap()
                .contains(":null")
        );
        assert_eq!(
            result["facets"]["tests"]["edges"].as_array().unwrap().len(),
            1
        );
        validate_stdout(&result).unwrap();
    }

    #[test]
    fn declared_identifier_beats_a_popular_analogue_without_raising_the_cap() {
        let declaration = |name: &str, file: &str, end_line: u64| {
            json!({
                "compilation":"cargo:demo",
                "factKey":format!("fact:{name}"),
                "payload":{
                    "kind":"declaration",
                    "name":name,
                    "symbolIdentity":format!("symbol:{name}"),
                    "file":file,
                    "startLine":1,
                    "endLine":end_line,
                }
            })
        };
        let retained = json!({
            "matches":[
                declaration("PopularAnalogue", "src/popular.rs", 4),
                declaration("PreciseTarget", "src/target.rs", 5),
                declaration("AlphaOnly", "src/alpha.rs", 3),
                declaration("BetaOnly", "src/beta.rs", 3),
            ],
            "sources":[
                {
                    "fileId":"src/popular.rs",
                    "contentRef":{"digest":"sha256:popular"},
                    "windows":[{"startLine":1,"endLine":4,"text":"fn PopularAnalogue() {\n alpha(); beta(); alpha(); beta();\n alpha(); beta();\n}"}]
                },
                {
                    "fileId":"src/target.rs",
                    "contentRef":{"digest":"sha256:target"},
                    "windows":[{"startLine":1,"endLine":5,"text":"fn PreciseTarget() {\n alpha();\n beta();\n gamma();\n}"}]
                },
                {
                    "fileId":"src/alpha.rs",
                    "contentRef":{"digest":"sha256:alpha"},
                    "windows":[{"startLine":1,"endLine":3,"text":"fn AlphaOnly() {\n alpha();\n}"}]
                },
                {
                    "fileId":"src/beta.rs",
                    "contentRef":{"digest":"sha256:beta"},
                    "windows":[{"startLine":1,"endLine":3,"text":"fn BetaOnly() {\n beta();\n}"}]
                }
            ],
            "completeness":{
                "status":"COMPLETE_TASK",
                "coverage":"QUERY_COMPLETE",
                "certainty":"VERIFIED",
                "unmatchedTerms":[]
            },
            "truncated":false
        });
        let result = assemble_with_decision_identifier(
            "session:adversarial",
            "context:adversarial",
            "sha256:evidence",
            NAV_QUERY_INTENT,
            &[
                "PreciseTarget".into(),
                "alpha".into(),
                "beta".into(),
                "gamma".into(),
            ],
            &retained,
            false,
            &[],
            Some("PreciseTarget"),
        )
        .unwrap();

        assert_eq!(result["candidates"][0]["displayName"], "PreciseTarget");
        assert_eq!(
            result["decisionAuthority"]["returnedCandidateCoverage"][0]["complete"],
            true
        );
        assert_eq!(
            result["candidateCount"],
            json!({"returned":3,"total":4,"omitted":1})
        );
        assert_eq!(result["decisionAuthority"]["status"], "SUPPORTED");
        assert_eq!(
            result["decisionAuthority"]["classification"],
            "UNIQUE_EXACT_IDENTIFIER_FULL_COVERAGE"
        );
        assert_eq!(
            result["decisionAuthority"]["candidateId"],
            result["candidates"][0]["candidateId"]
        );
        assert_eq!(result["queryCoverageTruncated"], false);
        assert_eq!(result["candidateListTruncated"], true);
        assert_eq!(result["truncated"], true);
        validate_stdout(&result).unwrap();

        let generic_only = assemble(
            "session:adversarial",
            "context:generic-only",
            "sha256:evidence",
            NAV_QUERY_INTENT,
            &["alpha".into(), "beta".into(), "gamma".into()],
            &retained,
            false,
            &[],
        )
        .unwrap();
        assert_eq!(generic_only["decisionAuthority"]["status"], "ABSTAIN");
        assert_eq!(
            generic_only["decisionAuthority"]["classification"],
            "NO_DECLARED_DECISION_IDENTIFIER"
        );

        let phrase = assemble_with_decision_identifier(
            "session:adversarial",
            "context:phrase",
            "sha256:evidence",
            NAV_QUERY_INTENT,
            &["PreciseTarget alpha-beta gamma".into()],
            &retained,
            false,
            &[],
            Some("PreciseTarget"),
        )
        .unwrap();
        assert_eq!(phrase["decisionAuthority"]["requiredTermCount"], 4);
        assert_eq!(phrase["decisionAuthority"]["status"], "SUPPORTED");
    }

    #[test]
    fn same_line_neighbor_cannot_complete_declared_identifier_coverage() {
        let retained = json!({
            "matches":[{
                "compilation":"tsconfig:app/tsconfig.json",
                "factKey":"fact:target",
                "payload":{
                    "kind":"declaration",
                    "name":"Target",
                    "symbolIdentity":"symbol:Target",
                    "file":"src/target.ts",
                    "startLine":1,
                    "endLine":1
                }
            }],
            "sources":[{
                "fileId":"src/target.ts",
                "contentRef":{"digest":"sha256:source"},
                "windows":[{
                    "startLine":1,
                    "endLine":1,
                    "text":"function Target() {} function Poll() {}"
                }]
            }],
            "completeness":{
                "status":"COMPLETE_TASK",
                "coverage":"QUERY_COMPLETE",
                "certainty":"VERIFIED",
                "unmatchedTerms":[]
            },
            "truncated":false
        });
        let result = assemble_with_decision_identifier(
            "session:same-line",
            "context:same-line",
            "sha256:evidence",
            NAV_QUERY_INTENT,
            &["Target".into(), "Poll".into()],
            &retained,
            false,
            &[],
            Some("Target"),
        )
        .unwrap();

        assert_eq!(result["decisionAuthority"]["status"], "ABSTAIN");
        assert_eq!(
            result["decisionAuthority"]["classification"],
            "PARTIAL_EXACT_IDENTIFIER_COVERAGE"
        );
        assert_eq!(
            result["decisionAuthority"]["refinement"]["kind"],
            "STOP_UNRESOLVED"
        );
        assert_eq!(
            result["decisionAuthority"]["refinement"]["repeatSameRequest"],
            false
        );
        assert!(
            result["decisionAuthority"]["refinement"]
                .get("command")
                .is_none()
        );
        assert_eq!(
            result["decisionAuthority"]["returnedCandidateCoverage"][0]["missingTermCount"],
            1
        );
    }

    #[test]
    fn declared_identifier_is_case_sensitive_and_never_inherited_from_owner() {
        let payload = |value: Value| value.as_object().unwrap().clone();

        assert!(candidate_matches_declared_identifier(
            &payload(json!({"name":"Target"})),
            "Target"
        ));
        assert!(!candidate_matches_declared_identifier(
            &payload(json!({"name":"target"})),
            "Target"
        ));
        assert!(!candidate_matches_declared_identifier(
            &payload(json!({"name":"child","ownerIdentity":"Target"})),
            "Target"
        ));
        assert!(candidate_matches_declared_identifier(
            &payload(json!({"qualifiedName":"pkg.Target"})),
            "pkg.Target"
        ));
        assert!(candidate_matches_declared_identifier(
            &payload(json!({"symbolIdentity":"symbol:Target"})),
            "symbol:Target"
        ));
    }

    #[test]
    fn decision_card_relevance_does_not_leak_from_a_distant_same_file_window() {
        let retained = json!({
            "matches":[
                {
                    "compilation":"tsconfig:app/tsconfig.json",
                    "factKey":"fact:fixture",
                    "payload":{
                        "kind":"declaration",
                        "name":"defaultProfileTopicsResponse",
                        "symbolIdentity":"ts:test.ts#variable:defaultProfileTopicsResponse",
                        "file":"src/DataToolsPage.test.tsx",
                        "startLine":28,
                        "endLine":58
                    }
                },
                {
                    "compilation":"tsconfig:app/tsconfig.json",
                    "factKey":"fact:refresh",
                    "payload":{
                        "kind":"declaration",
                        "name":"refreshProfileTopics",
                        "symbolIdentity":"ts:page.ts#function:refreshProfileTopics",
                        "file":"src/DataToolsPage.tsx",
                        "startLine":246,
                        "endLine":255
                    }
                }
            ],
            "sources":[
                {
                    "fileId":"src/DataToolsPage.test.tsx",
                    "contentRef":{"digest":"sha256:test"},
                    "windows":[
                        {
                            "startLine":28,
                            "endLine":58,
                            "text":"const defaultProfileTopicsResponse = { topics: [] }"
                        },
                        {
                            "startLine":488,
                            "endLine":490,
                            "text":"it('preserves deselected topics on manual refresh', () => {})"
                        }
                    ]
                },
                {
                    "fileId":"src/DataToolsPage.tsx",
                    "contentRef":{"digest":"sha256:page"},
                    "windows":[{
                        "startLine":246,
                        "endLine":255,
                        "text":"async function refreshProfileTopics() { return loadProfileTopics() }"
                    }]
                }
            ],
            "completeness":{},
            "truncated":false
        });
        let result = assemble(
            "session:test",
            "context:test",
            "sha256:evidence",
            NAV_QUERY_INTENT,
            &[
                "manual".into(),
                "refresh".into(),
                "deselected".into(),
                "topics".into(),
                "profile".into(),
            ],
            &retained,
            false,
            &[],
        )
        .unwrap();
        assert_eq!(
            result["candidates"][0]["displayName"],
            "refreshProfileTopics"
        );
    }

    #[test]
    fn decision_card_source_score_cannot_leak_across_a_merged_window() {
        let retained = json!({
            "matches":[
                {
                    "compilation":"cargo:demo",
                    "factKey":"fact:neighbor",
                    "payload":{
                        "kind":"declaration",
                        "name":"Neighbor",
                        "symbolIdentity":"symbol:Neighbor",
                        "file":"src/lib.rs",
                        "startLine":4,
                        "endLine":6
                    }
                },
                {
                    "compilation":"cargo:demo",
                    "factKey":"fact:target",
                    "payload":{
                        "kind":"declaration",
                        "name":"Target",
                        "symbolIdentity":"symbol:Target",
                        "file":"src/lib.rs",
                        "startLine":1,
                        "endLine":3
                    }
                }
            ],
            "sources":[{
                "fileId":"src/lib.rs",
                "contentRef":{"digest":"sha256:source"},
                "windows":[{
                    "startLine":1,
                    "endLine":6,
                    "text":"fn Target() {\n alpha(beta, gamma);\n}\nfn Neighbor() {\n unrelated();\n}"
                }]
            }],
            "completeness":{},
            "truncated":false,
        });
        let result = assemble(
            "session:test",
            "context:test",
            "sha256:evidence",
            NAV_QUERY_INTENT,
            &["alpha".into(), "beta".into(), "gamma".into()],
            &retained,
            false,
            &[],
        )
        .unwrap();
        assert_eq!(result["candidates"][0]["displayName"], "Target");
        assert_eq!(
            candidate_relevance(
                retained["matches"][0]["payload"].as_object().unwrap(),
                retained["sources"].as_array().unwrap(),
                &BTreeSet::from(["alpha".into(), "beta".into(), "gamma".into()]),
            ),
            (0, 0, 0)
        );
    }

    #[test]
    fn accepts_compiler_declaration_shape_without_guessing_a_name() {
        let projection = json!({
            "matches":[{
                "compilation":"cargo:Cargo.toml#demo#lib#demo",
                "factKey":"fact:java",
                "domainUri":"analysis:java-compiler-facts",
                "payload":{
                    "kind":"DECLARATION",
                    "declarationKind":"METHOD",
                    "symbolIdentity":"pkg.Target#run()V",
                    "file":"src/Target.java",
                    "start":100,
                    "end":120
                }
            }],
            "sources":[{"fileId":"src/Target.java","startLine":3,"endLine":8,"text":"source"}],
            "completeness":{},
            "truncated":false
        });
        let result = assemble(
            "session:test",
            "context:test",
            "sha256:evidence",
            NAV_QUERY_INTENT,
            &["run".into()],
            &projection,
            false,
            &[],
        )
        .unwrap();
        assert_eq!(result["candidates"][0]["displayName"], "pkg.Target#run()V");
        assert_eq!(result["candidates"][0]["location"]["start"], 100);
    }

    #[test]
    fn requested_relation_facets_only_return_identity_bound_direct_facts() {
        let mut projection = projection();
        projection["matches"].as_array_mut().unwrap().extend([
            json!({
                "compilation":"cargo:Cargo.toml#demo#lib#demo",
                "factKey":"fact:caller",
                "domainUri":"analysis:compiler",
                "payload":{
                    "kind":"RELATION",
                    "relationKind":"CALL",
                    "sourceIdentity":"symbol:Caller",
                    "targetIdentity":"symbol:Target"
                }
            }),
            json!({
                "compilation":"cargo:Cargo.toml#demo#lib#demo",
                "factKey":"fact:namesake-relation",
                "domainUri":"analysis:compiler",
                "payload":{
                    "kind":"RELATION",
                    "relationKind":"CALL",
                    "sourceIdentity":"symbol:Other",
                    "targetIdentity":"symbol:TargetNamesake"
                }
            }),
        ]);
        let result = assemble(
            "session:test",
            "context:test",
            "sha256:evidence",
            NAV_QUERY_INTENT,
            &["Target".into()],
            &projection,
            false,
            &[NavigationFacet::Callers, NavigationFacet::Tests],
        )
        .unwrap();
        assert_eq!(result["facets"]["callers"]["status"], "PARTIAL");
        assert_eq!(
            result["facets"]["callers"]["edges"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            result["facets"]["callers"]["edges"][0]["factKey"],
            "fact:caller"
        );
        assert_eq!(result["facets"]["tests"]["status"], "UNSUPPORTED");
        assert_eq!(result["facets"]["callees"]["status"], "NOT_REQUESTED");
    }

    #[test]
    fn refuses_navigation_stdout_above_the_public_bound() {
        let value = json!({"text":"x".repeat(MAX_NAV_STDOUT_BYTES)});
        let error = validate_stdout(&value).unwrap_err();
        assert_eq!(error.code, ErrorCode::SliceBudgetExceeded);
    }

    fn context(
        id: &str,
        parent: Option<&str>,
        evidence_digest: &str,
        projection: Value,
    ) -> ContextObject {
        let query_truncated = projection["truncated"].as_bool().unwrap_or(false);
        let retained = projection.clone();
        ContextObject {
            schema: "codeclew-context/4.0".into(),
            context_id: id.into(),
            session_id: "session:test".into(),
            session_authority_digest: "sha256:session".into(),
            parent_context_id: parent.map(str::to_owned),
            intent: NAV_QUERY_INTENT.into(),
            terms: vec![id.into()],
            evidence_digest: evidence_digest.into(),
            evidence_ref: crate::cas::CasObject {
                schema: "codeclew-cas-object/2.0".into(),
                object_schema: "codeclew-context-evidence/4.0".into(),
                digest: evidence_digest.into(),
                size: 1,
            },
            projection,
            evidence: json!({
                "context":retained,
                "queryContext":{"truncated":query_truncated},
            }),
        }
    }

    fn apply_candidate_delta(parent: &Value, delta: &Value) -> Vec<Value> {
        let mut reconstructed = candidate_map(parent)
            .unwrap()
            .into_iter()
            .map(|(key, value)| (key, value.clone()))
            .collect::<BTreeMap<_, _>>();
        for removal in delta["candidateDelta"]["removals"].as_array().unwrap() {
            reconstructed.remove(&(
                removal["compilation"].as_str().unwrap().to_owned(),
                removal["factKey"].as_str().unwrap().to_owned(),
            ));
        }
        for upsert in delta["candidateDelta"]["upserts"].as_array().unwrap() {
            let mut candidate = upsert.clone();
            candidate.as_object_mut().unwrap().remove("change");
            reconstructed.insert(candidate_key(&candidate).unwrap(), candidate);
        }
        delta["candidateDelta"]["candidateOrder"]
            .as_array()
            .unwrap()
            .iter()
            .map(|key| {
                reconstructed
                    .get(&(
                        key["compilation"].as_str().unwrap().to_owned(),
                        key["factKey"].as_str().unwrap().to_owned(),
                    ))
                    .unwrap()
                    .clone()
            })
            .collect()
    }

    #[test]
    fn expand_delta_omits_byte_identical_parent_candidates() {
        let parent = context("context:parent", None, "sha256:parent", projection());
        let child = context(
            "context:child",
            Some("context:parent"),
            "sha256:child",
            projection(),
        );
        let result = expand_delta(&parent, &child, &["new-term".into()], &[]).unwrap();
        assert_eq!(result["schema"], NAV_DELTA_SCHEMA);
        assert!(
            result["candidateDelta"]["upserts"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(
            result["candidateDelta"]["removals"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert_eq!(result["candidateDelta"]["unchangedCount"], 1);
        assert_eq!(result["parentEvidenceDigest"], "sha256:parent");
        assert_eq!(result["evidenceDigest"], "sha256:child");
        assert_eq!(
            result["decisionAuthority"]["schema"],
            NAV_DECISION_AUTHORITY_SCHEMA
        );
        assert_eq!(result["decisionAuthority"]["status"], "ABSTAIN");
    }

    #[test]
    fn expand_delta_reports_updates_additions_and_evictions() {
        let parent_projection = projection();
        let mut child_projection = projection();
        child_projection["matches"][0]["payload"]["endLine"] = json!(15);
        child_projection["matches"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "compilation":"cargo:Cargo.toml#demo#lib#demo",
                "factKey":"fact:added",
                "domainUri":"analysis:syntax",
                "payload":{
                    "kind":"declaration",
                    "name":"Added",
                    "declarationKind":"function",
                    "symbolIdentity":"symbol:Added",
                    "file":"src/added.rs",
                    "startLine":1,
                    "endLine":2
                }
            }));
        child_projection["sources"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "fileId":"src/added.rs","startLine":1,"endLine":2,"text":"added"
            }));
        let parent = context("context:parent", None, "sha256:parent", parent_projection);
        let child = context(
            "context:child",
            Some("context:parent"),
            "sha256:child",
            child_projection,
        );
        let result = expand_delta(&parent, &child, &["Added".into()], &[]).unwrap();
        let upserts = result["candidateDelta"]["upserts"].as_array().unwrap();
        assert_eq!(upserts.len(), 2);
        assert_eq!(upserts[0]["change"], "ADDED");
        assert_eq!(upserts[1]["change"], "UPDATED");
        let parent_view = query(&parent, &[]).unwrap();
        let child_view = query(&child, &[]).unwrap();
        let expected = child_view["candidates"].as_array().unwrap().clone();
        assert_eq!(apply_candidate_delta(&parent_view, &result), expected);

        let mut evicted_projection = child.projection.clone();
        evicted_projection["matches"] = json!([]);
        let evicted = context(
            "context:evicted",
            Some("context:child"),
            "sha256:evicted",
            evicted_projection,
        );
        let removal = expand_delta(&child, &evicted, &["other".into()], &[]).unwrap();
        assert_eq!(
            removal["candidateDelta"]["removals"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn expand_delta_preserves_reordered_unchanged_candidates() {
        let mut parent_projection = projection();
        parent_projection["matches"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "compilation":"cargo:Cargo.toml#demo#lib#demo",
                "factKey":"fact:second",
                "domainUri":"analysis:syntax",
                "payload":{
                    "kind":"declaration",
                    "name":"Second",
                    "declarationKind":"function",
                    "symbolIdentity":"symbol:Second",
                    "file":"src/lib.rs",
                    "startLine":20,
                    "endLine":21
                }
            }));
        let mut child_projection = parent_projection.clone();
        child_projection["matches"]
            .as_array_mut()
            .unwrap()
            .reverse();
        let parent = context("context:parent", None, "sha256:parent", parent_projection);
        let child = context(
            "context:child",
            Some("context:parent"),
            "sha256:child",
            child_projection,
        );
        let delta = expand_delta(&parent, &child, &["Second".into()], &[]).unwrap();
        assert!(
            delta["candidateDelta"]["upserts"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(
            delta["candidateDelta"]["removals"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert_eq!(delta["candidateDelta"]["unchangedCount"], 2);
        let parent_view = query(&parent, &[]).unwrap();
        let child_view = query(&child, &[]).unwrap();
        assert_eq!(
            apply_candidate_delta(&parent_view, &delta),
            child_view["candidates"].as_array().unwrap().clone()
        );
    }

    #[test]
    fn expand_delta_reconstructs_top_three_boundary_eviction() {
        let declarations = ["A", "B", "C", "D"]
            .into_iter()
            .map(|name| {
                json!({
                    "compilation":"cargo:demo",
                    "factKey":format!("fact:{name}"),
                    "payload":{
                        "kind":"declaration",
                        "name":name,
                        "symbolIdentity":format!("symbol:{name}"),
                        "file":"src/lib.rs",
                        "startLine":1,
                        "endLine":1,
                    }
                })
            })
            .collect::<Vec<_>>();
        let base = |matches: Vec<Value>| {
            json!({
                "matches":matches,
                "sources":[],
                "completeness":{},
                "truncated":false,
            })
        };
        let parent = context(
            "context:parent",
            None,
            "sha256:parent",
            base(declarations.clone()),
        );
        let child = context(
            "context:child",
            Some("context:parent"),
            "sha256:child",
            base(vec![
                declarations[3].clone(),
                declarations[0].clone(),
                declarations[1].clone(),
                declarations[2].clone(),
            ]),
        );
        let delta = expand_delta(&parent, &child, &["D".into()], &[]).unwrap();
        assert_eq!(
            delta["candidateDelta"]["upserts"].as_array().unwrap().len(),
            1
        );
        assert_eq!(delta["candidateDelta"]["upserts"][0]["change"], "ADDED");
        assert_eq!(
            delta["candidateDelta"]["upserts"][0]["candidateKey"]["factKey"],
            "fact:D"
        );
        assert_eq!(
            delta["candidateDelta"]["removals"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(delta["candidateDelta"]["removals"][0]["factKey"], "fact:C");
        assert_eq!(delta["candidateDelta"]["unchangedCount"], 2);
        let parent_view = query(&parent, &[]).unwrap();
        let child_view = query(&child, &[]).unwrap();
        assert_eq!(
            apply_candidate_delta(&parent_view, &delta),
            child_view["candidates"].as_array().unwrap().clone()
        );
    }

    #[test]
    fn expand_delta_keys_equal_fact_keys_by_compilation() {
        let parent = context("context:parent", None, "sha256:parent", projection());
        let mut child_projection = projection();
        child_projection["matches"][0]["compilation"] = json!("cargo:other#lib#other");
        let child = context(
            "context:child",
            Some("context:parent"),
            "sha256:child",
            child_projection,
        );
        let result = expand_delta(&parent, &child, &["Target".into()], &[]).unwrap();
        assert_eq!(
            result["candidateDelta"]["upserts"][0]["candidateKey"]["compilation"],
            "cargo:other#lib#other"
        );
        assert_eq!(
            result["candidateDelta"]["removals"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn expand_delta_keeps_requested_facets_and_rejects_unrelated_contexts() {
        let parent = context("context:parent", None, "sha256:parent", projection());
        let mut child_projection = projection();
        child_projection["matches"].as_array_mut().unwrap().extend([
            json!({
                "compilation":"cargo:Cargo.toml#demo#lib#demo",
                "factKey":"fact:caller",
                "domainUri":"analysis:compiler",
                "payload":{
                    "kind":"RELATION",
                    "relationKind":"CALL",
                    "sourceIdentity":"symbol:Caller",
                    "targetIdentity":"symbol:Target"
                }
            }),
            json!({
                "compilation":"cargo:other#lib#other",
                "factKey":"fact:caller",
                "domainUri":"analysis:compiler",
                "payload":{
                    "kind":"RELATION",
                    "relationKind":"CALL",
                    "sourceIdentity":"symbol:OtherCaller",
                    "targetIdentity":"symbol:Target"
                }
            }),
        ]);
        child_projection["truncated"] = json!(true);
        let child = context(
            "context:child",
            Some("context:parent"),
            "sha256:child",
            child_projection,
        );
        let result = expand_delta(
            &parent,
            &child,
            &["caller".into()],
            &[NavigationFacet::Callers],
        )
        .unwrap();
        assert_eq!(result["facets"]["callers"]["status"], "PARTIAL");
        let edges = result["facets"]["callers"]["edges"].as_array().unwrap();
        assert_eq!(edges.len(), 2);
        assert_ne!(edges[0]["edgeKey"], edges[1]["edgeKey"]);
        assert_eq!(result["truncated"], true);

        let unrelated = context("context:unrelated", None, "sha256:unrelated", projection());
        assert!(expand_delta(&unrelated, &child, &["caller".into()], &[]).is_err());
    }
}
