use crate::canonical;
use crate::error::{ClewError, ErrorCode};
use crate::session::ContextObject;
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub const NAV_QUERY_SCHEMA: &str = "codeclew-nav-query/2.0";
pub const NAV_EXPAND_SCHEMA: &str = "codeclew-nav-expand/2.0";
pub const NAV_RESULT_SCHEMA: &str = "codeclew-navigation-result/2.0";
pub const NAV_DELTA_SCHEMA: &str = "codeclew-navigation-delta/2.0";
pub const NAV_DETAIL_SCHEMA: &str = "codeclew-navigation-detail/1.0";
pub const NAV_QUERY_INTENT: &str = "NAVIGATION_QUERY";
pub const MAX_NAV_STDOUT_BYTES: usize = 64 * 1024;
const MAX_NAV_CANDIDATES: usize = 3;
const MAX_TERM_ANCHORS: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum NavigationFacet {
    Callers,
    Callees,
    Tests,
}

pub fn query(context: &ContextObject, facets: &[NavigationFacet]) -> Result<Value, ClewError> {
    let retained = retained_context(context)?;
    let result = assemble(
        &context.session_id,
        &context.context_id,
        &context.evidence_digest,
        &context.intent,
        &context.terms,
        retained,
        retained_query_truncated(context),
        facets,
    )?;
    validate_stdout(&result)?;
    Ok(result)
}

pub fn expand_delta(
    parent: &ContextObject,
    child: &ContextObject,
    requested_terms: &[String],
    facets: &[NavigationFacet],
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
    let child_view = assemble(
        &child.session_id,
        &child.context_id,
        &child.evidence_digest,
        &child.intent,
        &child.terms,
        retained_context(child)?,
        retained_query_truncated(child),
        facets,
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
        "completeness":child_view["completeness"],
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
    validate_file_selector(file)?;
    if term.is_empty() {
        return Err(invalid("exact navigation term is empty"));
    }
    let retained = retained_context(context)?;
    let matches = retained
        .get("matches")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("retained navigation context has no match array"))?;
    let mut candidate_ids = Vec::new();
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
        candidate_ids.push(candidate_handle(
            required_string(matched, "compilation")?,
            required_string(matched, "factKey")?,
        )?);
    }
    match candidate_ids.len() {
        0 => Err(ClewError::new(
            ErrorCode::SymbolNotFound,
            "no exact declaration matches the selected file and term",
        )),
        1 => detail(context, &candidate_ids, include_source, &[]),
        _ => Err(ClewError::new(
            ErrorCode::AmbiguousSymbol,
            "multiple exact declarations match the selected file and term",
        )),
    }
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
    Ok(json!({
        "candidateId":candidate_id,
        "source":exact_source_detail(sources, payload),
    }))
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
        "facets":{
            "callers":relation_facet(matches, &identities, NavigationFacet::Callers, requested_facets.contains(&NavigationFacet::Callers)),
            "callees":relation_facet(matches, &identities, NavigationFacet::Callees, requested_facets.contains(&NavigationFacet::Callees)),
            "tests":relation_facet(matches, &identities, NavigationFacet::Tests, requested_facets.contains(&NavigationFacet::Tests)),
        },
    }))
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
    let matches = retained
        .get("matches")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("retained navigation context has no match array"))?;
    let sources = retained
        .get("sources")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("retained navigation context has no source array"))?;

    let normalized_terms = terms
        .iter()
        .map(|term| term.to_lowercase())
        .filter(|term| !term.is_empty())
        .collect::<BTreeSet<_>>();
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
        let (term_coverage, name_coverage, occurrences) =
            candidate_relevance(payload, sources, &normalized_terms);
        ranked_candidates.push((
            term_coverage,
            name_coverage,
            occurrences,
            ordinal,
            candidate_id,
            matched,
            payload,
        ));
    }
    ranked_candidates.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| right.1.cmp(&left.1))
            .then_with(|| right.2.cmp(&left.2))
            .then_with(|| left.3.cmp(&right.3))
    });
    let total_candidates = ranked_candidates.len();
    let mut candidates = Vec::new();
    let mut candidate_identities = BTreeSet::new();
    for (_, _, _, _, candidate_id, matched, payload) in
        ranked_candidates.into_iter().take(MAX_NAV_CANDIDATES)
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
    });
    Ok(result)
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
        let rendered = canonical::compact(&selected).unwrap();
        assert_eq!(rendered.matches("TARGET_MARKER").count(), 1);
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
        let selected = source_by_candidate(&authority, candidate_id).unwrap();
        let rendered = canonical::compact(&selected).unwrap();
        assert!(rendered.len() < 2048);
        assert!(!rendered.contains("opaque"));
        assert_eq!(selected["source"]["windows"][0]["text"], "fn Large() {}");
        validate_stdout(&selected).unwrap();
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
                        "startLine":2,
                        "endLine":2
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
