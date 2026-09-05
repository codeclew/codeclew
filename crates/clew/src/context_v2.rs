use crate::canonical;
use crate::cas::{CasObject, CasStore};
use crate::error::{ClewError, ErrorCode};
use crate::generation_service::{
    ensure_session_generation, load_query_index, load_session_generation, load_snapshot,
};
use crate::query_v2::{FactHit, QueryContext, exact_name_query, expand, query};
use crate::repository_snapshot::{RepositoryInputSnapshot, WorktreeKind};
use crate::session::{ContextObject, SessionAuthority};
use crate::state::StateAuthority;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

const MAX_EVIDENCE_FACTS: usize = 4096;
const MAX_PAYLOAD_BYTES: usize = 1024 * 1024;
const MAX_SNIPPET_BYTES: usize = 16 * 1024;
const MAX_SOURCE_BYTES: usize = 32 * 1024;
const MAX_COMPLETE_FILE_SOURCE_BYTES: usize = 8 * 1024;
const MAX_SOURCE_WINDOWS: usize = 4;
const SOURCE_DECLARATION_CONTEXT_BEFORE_LINES: usize = 8;
const SOURCE_DECLARATION_CONTEXT_AFTER_LINES: usize = 24;
const MAX_CONTEXTUAL_DECLARATION_LINES: usize = 16;
const MAX_SUPPORT_TERM_SETS: usize = 32;
const PROJECTION_TARGET_BYTES: usize = 54 * 1024;
const MAX_EXACT_SELECTIONS: usize = 3;
const MAX_EXACT_EXPANSION_MATCHES_PER_TERM: usize = 4;
pub const AGGREGATE_QUERY_CONTEXT_SCHEMA: &str = "codeclew-aggregate-query-context/1.0";
pub const BOUNDED_CONTEXT_SCHEMA: &str = "codeclew-bounded-context/4.0";
pub const BOUNDED_CONTEXT_EVIDENCE_SCHEMA: &str = "codeclew-bounded-context-evidence/4.0";
pub const BOUNDED_CONTEXT_PROJECTION_SCHEMA: &str = "codeclew-bounded-context-projection/4.0";
const EXACT_SELECTION_SCHEMA: &str = "codeclew-exact-declaration-selection/1.0";
const EXACT_EXPANSION_SELECTION_SCHEMA: &str = "codeclew-exact-expansion-selection/1.0";

#[derive(Debug, Clone, Copy)]
struct ExactFileTermsSelector<'a> {
    file: &'a str,
    terms: &'a [String],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ExactSelectionAuthority {
    schema: String,
    file: String,
    term: String,
    compilation: String,
    fact_key: String,
    direct_posting_complete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ExactExpansionSelectionAuthority {
    schema: String,
    term: String,
    compilation: String,
    fact_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CompilationFactHit {
    compilation: String,
    fact: FactHit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AggregateQueryContext {
    schema: String,
    index_id: String,
    requested_terms: Vec<String>,
    unmatched_terms: Vec<String>,
    facts: Vec<CompilationFactHit>,
    query_shards_read: u32,
    truncated: bool,
}

pub fn validate_context_payload(projection: &Value, evidence: &Value) -> Result<(), ClewError> {
    if projection.get("schema").and_then(Value::as_str) != Some(BOUNDED_CONTEXT_PROJECTION_SCHEMA)
        || evidence.get("schema").and_then(Value::as_str) != Some(BOUNDED_CONTEXT_EVIDENCE_SCHEMA)
        || evidence.pointer("/context/schema").and_then(Value::as_str)
            != Some(BOUNDED_CONTEXT_SCHEMA)
    {
        return Err(invalid("multi-compilation context schema is invalid"));
    }
    let aggregate: AggregateQueryContext = serde_json::from_value(
        evidence
            .get("queryContext")
            .cloned()
            .ok_or_else(|| invalid("context has no aggregate query authority"))?,
    )
    .map_err(parse_error)?;
    if aggregate.schema != AGGREGATE_QUERY_CONTEXT_SCHEMA {
        return Err(invalid("aggregate query context schema is invalid"));
    }
    let queries: BTreeMap<String, QueryContext> = serde_json::from_value(
        evidence
            .get("queryContexts")
            .cloned()
            .ok_or_else(|| invalid("context has no per-compilation query authority"))?,
    )
    .map_err(parse_error)?;
    let exact_selections = exact_selection_authorities(evidence)?;
    let exact_expansion_selections = exact_expansion_selection_authorities(evidence)?;
    if !exact_selections.is_empty() && !exact_expansion_selections.is_empty() {
        return Err(invalid("exact selection authorities are inconsistent"));
    }
    let compilations = evidence
        .pointer("/context/compilations")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("context has no compilation authority"))?;
    let compilation_set = compilations
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| invalid("context compilation authority is invalid"))
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    let language = evidence
        .pointer("/context/language")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("context has no language authority"))?;
    if !supported_context_language(language)
        || compilation_set.is_empty()
        || compilation_set.len() != compilations.len()
        || queries.keys().ne(compilation_set.iter())
        || projection.get("compilations") != evidence.pointer("/context/compilations")
        || projection.get("language").and_then(Value::as_str) != Some(language)
        || queries.values().any(|query| {
            query.schema != crate::query_v2::QUERY_CONTEXT_SCHEMA
                || query.requested_terms != aggregate.requested_terms
        })
        || aggregate
            .facts
            .iter()
            .any(|hit| !compilation_set.contains(&hit.compilation))
    {
        return Err(invalid(
            "context compilation or query authority is inconsistent",
        ));
    }
    let retained = evidence
        .get("context")
        .ok_or_else(|| invalid("context evidence has no retained context"))?;
    for key in [
        "language",
        "snapshot",
        "task",
        "compilations",
        "compilerVersions",
        "projectCompilerVersions",
        "generationAuthority",
        "completeness",
        "verificationObligations",
    ] {
        if projection.get(key) != retained.get(key) {
            return Err(invalid(
                "context projection authority differs from evidence",
            ));
        }
    }
    let omitted_matches = validate_projected_subset(projection, retained, "matches")?;
    let omitted_sources = validate_projected_subset(projection, retained, "sources")?;
    if projection.get("truncated").and_then(Value::as_bool)
        != Some(omitted_matches || omitted_sources)
    {
        return Err(invalid(
            "context projection truncation does not match retained omissions",
        ));
    }
    let required = exact_selections
        .iter()
        .map(|selection| exact_selection_hit(&queries, selection))
        .collect::<Result<Vec<_>, _>>()?;
    let expansion_required = exact_expansion_selections
        .iter()
        .map(|selection| exact_expansion_selection_hit(&queries, selection))
        .collect::<Result<Vec<_>, _>>()?;
    let mut aggregate_required = required.clone();
    aggregate_required.extend(expansion_required.iter().cloned());
    let recomputed = merge_query_contexts_with_required(
        &queries,
        aggregate.facts.len().max(1),
        &aggregate_required,
    )?;
    if aggregate != recomputed {
        return Err(invalid("aggregate query authority cannot be reproduced"));
    }
    for matches in [
        evidence.pointer("/context/matches"),
        projection.get("matches"),
    ] {
        if matches
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .any(|value| {
                value
                    .get("compilation")
                    .and_then(Value::as_str)
                    .is_none_or(|compilation| !compilation_set.contains(compilation))
            })
        {
            return Err(invalid("context fact has no compilation provenance"));
        }
    }
    validate_source_rows(
        evidence
            .pointer("/context/sources")
            .ok_or_else(|| invalid("context has no source authority"))?,
    )?;
    validate_source_rows(
        projection
            .get("sources")
            .ok_or_else(|| invalid("projection has no source authority"))?,
    )?;
    for (selection, required) in exact_selections.iter().zip(&required) {
        validate_exact_selection(selection, retained, required)?;
    }
    for (selection, required) in exact_expansion_selections.iter().zip(&expansion_required) {
        validate_exact_expansion_selection(
            selection,
            retained,
            required,
            &aggregate.requested_terms,
        )?;
    }
    Ok(())
}

fn exact_selection_authorities(
    evidence: &Value,
) -> Result<Vec<ExactSelectionAuthority>, ClewError> {
    let single = evidence.get("exactSelection");
    let batch = evidence.get("exactSelections");
    if single.is_some() && batch.is_some() {
        return Err(invalid("exact selection authority is duplicated"));
    }
    let selections = if let Some(value) = single {
        vec![
            serde_json::from_value::<ExactSelectionAuthority>(value.clone())
                .map_err(parse_error)?,
        ]
    } else if let Some(value) = batch {
        let selections = serde_json::from_value::<Vec<ExactSelectionAuthority>>(value.clone())
            .map_err(parse_error)?;
        if !(2..=MAX_EXACT_SELECTIONS).contains(&selections.len()) {
            return Err(invalid("exact selection batch size is invalid"));
        }
        selections
    } else {
        Vec::new()
    };
    let mut terms = BTreeSet::new();
    let mut facts = BTreeSet::new();
    let mut file = None;
    for selection in &selections {
        if !terms.insert(selection.term.as_str())
            || !facts.insert((selection.compilation.as_str(), selection.fact_key.as_str()))
            || file.is_some_and(|file| file != selection.file.as_str())
        {
            return Err(invalid("exact selection batch authority is inconsistent"));
        }
        file = Some(selection.file.as_str());
    }
    Ok(selections)
}

fn exact_expansion_selection_authorities(
    evidence: &Value,
) -> Result<Vec<ExactExpansionSelectionAuthority>, ClewError> {
    let Some(value) = evidence.get("exactExpansionSelections") else {
        return Ok(Vec::new());
    };
    let selections = serde_json::from_value::<Vec<ExactExpansionSelectionAuthority>>(value.clone())
        .map_err(parse_error)?;
    if selections.is_empty()
        || selections.len()
            > MAX_EXACT_SELECTIONS.saturating_mul(MAX_EXACT_EXPANSION_MATCHES_PER_TERM)
    {
        return Err(invalid("exact expansion selection count is invalid"));
    }
    let mut facts = BTreeSet::new();
    for selection in &selections {
        if selection.schema != EXACT_EXPANSION_SELECTION_SCHEMA
            || selection.term.is_empty()
            || !facts.insert((selection.compilation.as_str(), selection.fact_key.as_str()))
        {
            return Err(invalid("exact expansion selection authority is invalid"));
        }
    }
    Ok(selections)
}

fn validate_projected_subset(
    projection: &Value,
    retained: &Value,
    key: &str,
) -> Result<bool, ClewError> {
    let projected = projection
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("context projection array is missing"))?;
    let retained = retained
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("retained context array is missing"))?;
    let mut seen = BTreeSet::new();
    let mut retained_cursor = 0usize;
    for value in projected {
        let digest = canonical::hash(value).map_err(internal)?;
        let Some(offset) = retained[retained_cursor..]
            .iter()
            .position(|candidate| candidate == value)
        else {
            return Err(invalid(
                "context projection is not an exact subset of retained evidence",
            ));
        };
        if !seen.insert(digest) {
            return Err(invalid("context projection contains duplicate evidence"));
        }
        retained_cursor += offset + 1;
    }
    Ok(projected.len() < retained.len())
}

fn supported_context_language(language: &str) -> bool {
    matches!(
        language,
        "language:java"
            | "language:javascript"
            | "language:kotlin"
            | "language:python"
            | "language:rust"
            | "language:typescript"
    )
}

fn exact_selection_hit(
    queries: &BTreeMap<String, QueryContext>,
    selection: &ExactSelectionAuthority,
) -> Result<CompilationFactHit, ClewError> {
    let query = queries
        .get(&selection.compilation)
        .ok_or_else(|| invalid("exact selection compilation is absent"))?;
    let mut facts = query
        .facts
        .iter()
        .filter(|fact| fact.fact_key == selection.fact_key);
    let fact = facts
        .next()
        .cloned()
        .ok_or_else(|| invalid("exact selection fact is absent"))?;
    if facts.next().is_some() {
        return Err(invalid("exact selection fact authority is duplicated"));
    }
    Ok(CompilationFactHit {
        compilation: selection.compilation.clone(),
        fact,
    })
}

fn exact_expansion_selection_hit(
    queries: &BTreeMap<String, QueryContext>,
    selection: &ExactExpansionSelectionAuthority,
) -> Result<CompilationFactHit, ClewError> {
    let query = queries
        .get(&selection.compilation)
        .ok_or_else(|| invalid("exact expansion selection compilation is absent"))?;
    let fact = query
        .facts
        .iter()
        .find(|fact| fact.fact_key == selection.fact_key)
        .cloned()
        .ok_or_else(|| invalid("exact expansion selection fact is absent"))?;
    Ok(CompilationFactHit {
        compilation: selection.compilation.clone(),
        fact,
    })
}

fn validate_exact_selection(
    selection: &ExactSelectionAuthority,
    retained: &Value,
    required: &CompilationFactHit,
) -> Result<(), ClewError> {
    validate_exact_file_selector(&selection.file)?;
    if selection.schema != EXACT_SELECTION_SCHEMA
        || selection.term.is_empty()
        || !selection.direct_posting_complete
        || required.compilation != selection.compilation
        || required.fact.fact_key != selection.fact_key
    {
        return Err(invalid("exact declaration selection authority is invalid"));
    }
    let matches = retained
        .get("matches")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("exact selection context has no matches"))?;
    let selected = matches
        .iter()
        .filter(|fact| {
            fact.get("compilation").and_then(Value::as_str) == Some(selection.compilation.as_str())
                && fact.get("factKey").and_then(Value::as_str) == Some(selection.fact_key.as_str())
        })
        .collect::<Vec<_>>();
    let expected_payload_ref = serde_json::to_value(&required.fact.payload).map_err(internal)?;
    if selected.len() != 1
        || selected[0].get("domainUri").and_then(Value::as_str)
            != Some(required.fact.domain_uri.as_str())
        || selected[0].get("payloadRef") != Some(&expected_payload_ref)
        || !exact_declaration_file_term_matches(
            &selected[0]["payload"],
            &selection.file,
            &selection.term,
        )
        || !exact_source_window_retained(
            retained.get("sources").and_then(Value::as_array),
            &selected[0]["payload"],
        )
    {
        return Err(invalid(
            "exact declaration selection is not retained with its source",
        ));
    }
    Ok(())
}

fn validate_exact_expansion_selection(
    selection: &ExactExpansionSelectionAuthority,
    retained: &Value,
    required: &CompilationFactHit,
    requested_terms: &[String],
) -> Result<(), ClewError> {
    if selection.schema != EXACT_EXPANSION_SELECTION_SCHEMA
        || !requested_terms.contains(&selection.term)
        || required.compilation != selection.compilation
        || required.fact.fact_key != selection.fact_key
    {
        return Err(invalid("exact expansion selection authority is invalid"));
    }
    let matches = retained
        .get("matches")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("exact expansion context has no matches"))?;
    let selected = matches
        .iter()
        .filter(|fact| {
            fact.get("compilation").and_then(Value::as_str) == Some(selection.compilation.as_str())
                && fact.get("factKey").and_then(Value::as_str) == Some(selection.fact_key.as_str())
        })
        .collect::<Vec<_>>();
    if selected.len() != 1
        || !exact_declaration_term_matches(&selected[0]["payload"], &selection.term)
    {
        return Err(invalid(
            "exact expansion selection is not retained with exact declaration authority",
        ));
    }
    Ok(())
}

fn exact_source_window_retained(sources: Option<&Vec<Value>>, payload: &Value) -> bool {
    let Some(payload) = payload.as_object() else {
        return false;
    };
    let Some(file) = payload.get("file").and_then(Value::as_str) else {
        return false;
    };
    let Some(declaration_start) = payload.get("startLine").and_then(Value::as_u64) else {
        return false;
    };
    let Some(declaration_end) = payload.get("endLine").and_then(Value::as_u64) else {
        return false;
    };
    declaration_start > 0
        && declaration_start <= declaration_end
        && sources.is_some_and(|sources| {
            sources
                .iter()
                .filter(|row| row.get("fileId").and_then(Value::as_str) == Some(file))
                .flat_map(|row| {
                    row.get("windows")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                })
                .any(|window| {
                    window
                        .get("startLine")
                        .and_then(Value::as_u64)
                        .is_some_and(|start| start <= declaration_start)
                        && window
                            .get("endLine")
                            .and_then(Value::as_u64)
                            .is_some_and(|end| declaration_end <= end)
                })
        })
}

fn exact_declaration_file_term_matches(payload: &Value, file: &str, term: &str) -> bool {
    exact_declaration_term_matches(payload, term)
        && payload.get("file").and_then(Value::as_str) == Some(file)
}

fn exact_declaration_term_matches(payload: &Value, term: &str) -> bool {
    let Some(payload) = payload.as_object() else {
        return false;
    };
    let declaration = payload
        .get("kind")
        .and_then(Value::as_str)
        .is_some_and(|kind| kind.eq_ignore_ascii_case("declaration"))
        || (payload.contains_key("declarationKind") && payload.contains_key("symbolIdentity"));
    declaration && crate::query_v2::declaration_identifiers(payload).contains(term)
}

fn validate_exact_file_selector(file: &str) -> Result<(), ClewError> {
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

fn validate_source_rows(value: &Value) -> Result<(), ClewError> {
    let rows = value
        .as_array()
        .ok_or_else(|| invalid("context sources are invalid"))?;
    let mut files = BTreeSet::new();
    for row in rows {
        let file = row
            .get("fileId")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("context source has no file identity"))?;
        let content: CasObject = serde_json::from_value(
            row.get("contentRef")
                .cloned()
                .ok_or_else(|| invalid("context source has no content authority"))?,
        )
        .map_err(parse_error)?;
        let windows = row
            .get("windows")
            .and_then(Value::as_array)
            .ok_or_else(|| invalid("context source has no windows"))?;
        if !safe_path(file)
            || !files.insert(file)
            || windows.is_empty()
            || windows.len() > MAX_SOURCE_WINDOWS
        {
            return Err(invalid("context source set authority is invalid"));
        }
        let mut previous_end = 0u64;
        let mut total = 0usize;
        let mut combined = String::new();
        for (index, window) in windows.iter().enumerate() {
            let start = window
                .get("startLine")
                .and_then(Value::as_u64)
                .ok_or_else(|| invalid("context source window start is invalid"))?;
            let end = window
                .get("endLine")
                .and_then(Value::as_u64)
                .ok_or_else(|| invalid("context source window end is invalid"))?;
            let text = window
                .get("text")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid("context source window text is invalid"))?;
            if start == 0 || end < start || start <= previous_end {
                return Err(invalid("context source windows overlap or are unordered"));
            }
            previous_end = end;
            total = total.saturating_add(text.len());
            if index > 0 {
                combined.push_str(&format!("\nCODECLEW_OMITTED_LINES_BEFORE_{start}\n"));
            }
            combined.push_str(text);
        }
        let complete = row
            .get("completeFile")
            .and_then(Value::as_bool)
            .ok_or_else(|| invalid("context source completeness is invalid"))?;
        if total > MAX_SOURCE_BYTES
            || row.get("startLine").and_then(Value::as_u64)
                != windows[0].get("startLine").and_then(Value::as_u64)
            || row.get("endLine").and_then(Value::as_u64)
                != windows
                    .last()
                    .and_then(|window| window.get("endLine"))
                    .and_then(Value::as_u64)
            || row.get("text").and_then(Value::as_str) != Some(combined.as_str())
            || (complete && (windows.len() != 1 || content.size != total as u64))
        {
            return Err(invalid("context source projection authority is invalid"));
        }
    }
    Ok(())
}

pub fn create(
    session: &SessionAuthority,
    intent: &str,
    terms: &[String],
    max_roots: usize,
    parent: Option<&ContextObject>,
) -> Result<(Value, Value), ClewError> {
    create_with_selector(session, intent, terms, max_roots, parent, None, false)
}

pub fn create_reference_follow(
    session: &SessionAuthority,
    intent: &str,
    terms: &[String],
    max_roots: usize,
    parent: &ContextObject,
) -> Result<(Value, Value), ClewError> {
    create_with_selector(session, intent, terms, max_roots, Some(parent), None, true)
}

pub fn create_exact_file_terms(
    session: &SessionAuthority,
    intent: &str,
    terms: &[String],
    max_roots: usize,
    parent: &ContextObject,
    file: &str,
) -> Result<(Value, Value), ClewError> {
    validate_exact_file_selector(file)?;
    if !(1..=MAX_EXACT_SELECTIONS).contains(&terms.len())
        || terms.iter().any(String::is_empty)
        || terms.iter().collect::<BTreeSet<_>>().len() != terms.len()
    {
        return Err(invalid(
            "exact context selection requires one to three unique declaration terms",
        ));
    }
    create_with_selector(
        session,
        intent,
        terms,
        max_roots,
        Some(parent),
        Some(ExactFileTermsSelector { file, terms }),
        false,
    )
}

fn create_with_selector(
    session: &SessionAuthority,
    intent: &str,
    terms: &[String],
    max_roots: usize,
    parent: Option<&ContextObject>,
    exact_selector: Option<ExactFileTermsSelector<'_>>,
    complete_small_sources: bool,
) -> Result<(Value, Value), ClewError> {
    crate::session::validate_context_request(intent, terms)?;
    if terms.is_empty() || max_roots == 0 || max_roots > 256 {
        return Err(invalid("bounded context terms or root limit is invalid"));
    }
    let ready = if parent.is_some() {
        load_session_generation(session)?
    } else {
        ensure_session_generation(session)?
    };
    let state = StateAuthority::process_default()?;
    let store = CasStore::open(&state)?;
    let fact_limit = max_roots.saturating_mul(16).min(MAX_EVIDENCE_FACTS);
    let exact_expansion_terms =
        (parent.is_some() && exact_selector.is_none() && terms.len() <= MAX_EXACT_SELECTIONS)
            .then_some(terms);
    let exact_expansion_match_limit = exact_expansion_terms.map_or(0, |terms| {
        max_roots
            .saturating_mul(4)
            .checked_div(terms.len())
            .unwrap_or(0)
            .clamp(1, MAX_EXACT_EXPANSION_MATCHES_PER_TERM)
    });
    let required_capacity = exact_selector.map_or_else(
        || {
            exact_expansion_terms.map_or(0, |terms| {
                terms.len().saturating_mul(exact_expansion_match_limit)
            })
        },
        |selector| selector.terms.len(),
    );
    let query_limit = fact_limit.saturating_sub(required_capacity).max(1);
    let parent_queries = parent
        .map(|parent| {
            serde_json::from_value::<BTreeMap<String, QueryContext>>(
                parent
                    .evidence
                    .get("queryContexts")
                    .cloned()
                    .ok_or_else(|| invalid("parent context has no compilation query authority"))?,
            )
            .map_err(parse_error)
        })
        .transpose()?;
    let mut query_contexts = BTreeMap::new();
    let mut exact_matches = BTreeMap::<String, BTreeSet<CompilationFactHit>>::new();
    let mut exact_query_truncated = false;
    for compilation in &ready.compilations {
        let index = load_query_index(&store, compilation)?;
        let mut context = if let Some(parent_queries) = &parent_queries {
            let parent_query = parent_queries
                .get(&compilation.compilation)
                .ok_or_else(|| invalid("parent context misses a selected compilation"))?;
            expand(&store, &index, parent_query, terms, query_limit)?
        } else {
            query(&store, &index, terms, query_limit)?
        };
        let exact_terms = exact_selector
            .map(|selector| selector.terms)
            .or(exact_expansion_terms);
        if let Some(exact_terms) = exact_terms {
            for term in exact_terms {
                let exact = exact_name_query(&store, &index, term)?;
                exact_query_truncated |= exact.truncated;
                if exact_selector.is_none() && exact.truncated {
                    context.truncated = true;
                }
                context.query_shards_read = context
                    .query_shards_read
                    .checked_add(exact.query_shards_read)
                    .ok_or_else(|| resource("context query shard count overflow"))?;
                for fact in exact.facts {
                    let payload = load_fact_payload(&store, &fact)?;
                    let selected = exact_selector.map_or_else(
                        || exact_declaration_term_matches(&payload, term),
                        |selector| {
                            exact_declaration_file_term_matches(&payload, selector.file, term)
                        },
                    );
                    if selected {
                        exact_matches
                            .entry(term.clone())
                            .or_default()
                            .insert(CompilationFactHit {
                                compilation: compilation.compilation.clone(),
                                fact,
                            });
                    }
                }
            }
        }
        query_contexts.insert(compilation.compilation.clone(), context);
    }
    let (exact_matches, bounded_exact_truncated) = if let Some(selector) = exact_selector {
        (
            selector
                .terms
                .iter()
                .map(|term| {
                    select_unique_exact_match(
                        exact_matches.remove(term).unwrap_or_default(),
                        exact_query_truncated,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?,
            false,
        )
    } else if let Some(terms) = exact_expansion_terms {
        select_bounded_exact_expansion_matches(
            &mut exact_matches,
            terms,
            exact_expansion_match_limit,
        )
    } else {
        (Vec::new(), false)
    };
    if bounded_exact_truncated {
        for context in query_contexts.values_mut() {
            context.truncated = true;
        }
    }
    for selected in exact_matches.iter().rev() {
        let context = query_contexts
            .get_mut(&selected.compilation)
            .ok_or_else(|| internal("exact declaration compilation disappeared"))?;
        promote_query_fact(context, &selected.fact, fact_limit);
    }
    let query_context =
        merge_query_contexts_with_required(&query_contexts, fact_limit, &exact_matches)?;
    let snapshot = load_snapshot(&store, &ready)?;
    let selection_terms = &query_context.requested_terms;
    let loaded_facts = load_fact_evidence(&store, &query_context.facts)?;
    let evidence_facts = if !exact_matches.is_empty() {
        rank_fact_evidence_with_required(
            loaded_facts,
            selection_terms,
            max_roots.saturating_mul(4),
            &exact_matches,
        )?
    } else {
        rank_fact_evidence(loaded_facts, selection_terms, max_roots.saturating_mul(4))?
    };
    let mut paths = ordered_paths_in_evidence(&evidence_facts);
    let semantic_paths = paths.iter().cloned().collect::<BTreeSet<_>>();
    let mut visible_paths = snapshot
        .index
        .iter()
        .filter(|entry| entry.stage == 0)
        .map(|entry| entry.path.clone())
        .collect::<BTreeSet<_>>();
    for entry in &snapshot.worktree {
        match entry.kind {
            WorktreeKind::Regular => {
                visible_paths.insert(entry.path.clone());
            }
            WorktreeKind::Missing | WorktreeKind::Symlink => {
                visible_paths.remove(&entry.path);
            }
        }
    }
    let mut lexical_candidates = visible_paths
        .into_iter()
        .filter(|path| !semantic_paths.contains(path))
        .filter_map(|path| {
            let file_name = path.rsplit('/').next().unwrap_or(path.as_str());
            let stem = file_name
                .rsplit_once('.')
                .map_or(file_name, |(stem, _)| stem)
                .to_lowercase();
            let matched_terms = query_context
                .unmatched_terms
                .iter()
                .filter(|term| stem.contains(term.as_str()))
                .cloned()
                .collect::<Vec<_>>();
            if matched_terms.is_empty() {
                None
            } else {
                let exact_matches = matched_terms
                    .iter()
                    .filter(|term| stem == term.as_str())
                    .count();
                Some((exact_matches, path, matched_terms))
            }
        })
        .collect::<Vec<_>>();
    lexical_candidates
        .sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
    lexical_candidates.truncate(max_roots);
    let lexical_fallback = lexical_candidates
        .into_iter()
        .map(|(_, path, terms)| (path, terms))
        .collect::<BTreeMap<_, _>>();
    let semantic_paths_in_order = std::mem::take(&mut paths);
    paths.extend(lexical_fallback.keys().cloned());
    paths.extend(semantic_paths_in_order);

    let source_hints = source_offset_hints(&evidence_facts, selection_terms);
    let mut sources = load_source_snippets(
        &store,
        &snapshot,
        &paths,
        selection_terms,
        &source_hints,
        complete_small_sources,
    )?;
    for source in &mut sources {
        let Some(file) = source.get("fileId").and_then(Value::as_str) else {
            continue;
        };
        let Some(matched_terms) = lexical_fallback.get(file) else {
            continue;
        };
        source
            .as_object_mut()
            .ok_or_else(|| invalid("context source row is invalid"))?
            .insert(
                "selectionAuthority".into(),
                json!({
                    "mode":"REPOSITORY_PATH_LEXICAL",
                    "certainty":"UNSURE",
                    "matchedTerms":matched_terms,
                }),
            );
    }
    let verified = query_selection_verified(&ready.certainty, &query_context, &evidence_facts);
    let conditional = !verified && !query_context.facts.is_empty();
    let mut obligations = ready
        .obligations
        .iter()
        .map(|obligation| json!({
            "id":obligation,
            "code":"UNSURE_GENERATION_AUTHORITY",
            "subject":query_context.requested_terms,
            "requiredCheckSet":["perform the named runtime or semantic verification before publication"],
            "publicationBlocking":true,
        }))
        .collect::<Vec<_>>();
    if !lexical_fallback.is_empty() {
        let lexical_terms = lexical_fallback
            .values()
            .flatten()
            .cloned()
            .collect::<BTreeSet<_>>();
        obligations.push(json!({
            "id":"verify-lexical-path-fallback",
            "code":"VERIFY_LEXICAL_SOURCE_SELECTION",
            "subject":lexical_terms,
            "requiredCheckSet":["confirm the selected tracked files are relevant and verify their semantic relationship to the task"],
            "publicationBlocking":true,
        }));
    }
    if conditional && obligations.is_empty() {
        obligations.push(json!({
            "id":"verify-query-selection",
            "code":"VERIFY_QUERY_SELECTION",
            "subject":query_context.requested_terms,
            "requiredCheckSet":["confirm exact declarations, callers, boundaries, and tests before publication"],
            "publicationBlocking":true,
        }));
    }
    let status = if query_context.facts.is_empty() {
        "INCOMPLETE"
    } else if verified {
        "COMPLETE_TASK"
    } else {
        "CONDITIONAL_TASK"
    };
    let certainty = if verified { "VERIFIED" } else { "UNSURE" };
    let project_compiler_versions =
        retained_project_compiler_versions(&store, session.language, &ready.compilations)?;
    let context = json!({
        "schema":BOUNDED_CONTEXT_SCHEMA,
        "language":session.language.uri(),
        "snapshot":{
            "baseRevision":session.base_revision,
            "snapshotId":snapshot.snapshot_id,
            "repositorySnapshot":ready.repository_snapshot,
            "compilations":ready.compilations.iter().map(|compilation| json!({
                "compilation":compilation.compilation,
                "compilerVersion":compilation.compiler_version,
                "generation":compilation.generation,
                "queryIndex":compilation.query_index,
            })).collect::<Vec<_>>(),
        },
        "task":{"intent":intent,"terms":query_context.requested_terms},
        "compilations":session.compilations,
        "compilerVersions":ready.compilations.iter().map(|compilation| {
            (compilation.compilation.clone(), compilation.compiler_version.clone())
        }).collect::<BTreeMap<_, _>>(),
        "projectCompilerVersions":project_compiler_versions,
        "generationAuthority":{
            "coverage":ready.coverage,
            "certainty":ready.certainty,
        },
        "matches":evidence_facts,
        "sources":sources,
        "completeness":{
            "status":status,
            "support":"SUPPORTED",
            "coverage":if query_context.truncated { "PARTIAL" } else { "QUERY_COMPLETE" },
            "certainty":certainty,
            "unmatchedTerms":query_context.unmatched_terms,
        },
        "verificationObligations":obligations,
        "publicationPolicy":{
            "mode":if conditional { "EXPLICIT_CONDITIONAL" } else { "STRICT" },
            "status":if verified {
                "READY"
            } else if conditional {
                "REQUIRES_EXPLICIT_ACKNOWLEDGEMENT_AND_VALIDATION"
            } else {
                "BLOCKED_INCOMPLETE"
            },
            "automaticPublication":verified,
        },
    });
    let projection = bounded_projection(&context)?;
    let mut evidence = json!({
        "schema":BOUNDED_CONTEXT_EVIDENCE_SCHEMA,
        "context":context,
        "queryContext":query_context,
        "queryContexts":query_contexts,
        "stdoutCompleteness":{
            "status":status,
            "certainty":certainty,
        },
    });
    if let Some(selector) = exact_selector {
        let selections = selector
            .terms
            .iter()
            .zip(&exact_matches)
            .map(|(term, selected)| ExactSelectionAuthority {
                schema: EXACT_SELECTION_SCHEMA.into(),
                file: selector.file.into(),
                term: term.clone(),
                compilation: selected.compilation.clone(),
                fact_key: selected.fact.fact_key.clone(),
                direct_posting_complete: true,
            })
            .collect::<Vec<_>>();
        let (key, value) = if selections.len() == 1 {
            (
                "exactSelection",
                serde_json::to_value(&selections[0]).map_err(internal)?,
            )
        } else {
            (
                "exactSelections",
                serde_json::to_value(&selections).map_err(internal)?,
            )
        };
        evidence
            .as_object_mut()
            .expect("context evidence is an object")
            .insert(key.into(), value);
    }
    if let Some(terms) = exact_expansion_terms
        && !exact_matches.is_empty()
    {
        let selections = exact_matches
            .iter()
            .map(|selected| {
                let payload = load_fact_payload(&store, &selected.fact)?;
                let term = terms
                    .iter()
                    .find(|term| exact_declaration_term_matches(&payload, term))
                    .ok_or_else(|| {
                        internal("exact expansion fact has no requested name authority")
                    })?;
                Ok(ExactExpansionSelectionAuthority {
                    schema: EXACT_EXPANSION_SELECTION_SCHEMA.into(),
                    term: term.into(),
                    compilation: selected.compilation.clone(),
                    fact_key: selected.fact.fact_key.clone(),
                })
            })
            .collect::<Result<Vec<_>, ClewError>>()?;
        evidence
            .as_object_mut()
            .expect("context evidence is an object")
            .insert(
                "exactExpansionSelections".into(),
                serde_json::to_value(selections).map_err(internal)?,
            );
    }
    Ok((projection, evidence))
}

fn query_selection_verified(
    generation_certainty: &str,
    query_context: &AggregateQueryContext,
    evidence_facts: &[Value],
) -> bool {
    if generation_certainty != "VERIFIED"
        || query_context.truncated
        || !query_context.unmatched_terms.is_empty()
        || evidence_facts.is_empty()
    {
        return false;
    }
    let exact_identities = evidence_facts
        .iter()
        .flat_map(|fact| exact_identity_terms(&fact["payload"]))
        .collect::<BTreeSet<_>>();
    query_context
        .requested_terms
        .iter()
        .all(|term| exact_identities.contains(term))
}

#[cfg(test)]
fn merge_query_contexts(
    contexts: &BTreeMap<String, QueryContext>,
    fact_limit: usize,
) -> Result<AggregateQueryContext, ClewError> {
    merge_query_contexts_with_required(contexts, fact_limit, &[])
}

fn merge_query_contexts_with_required(
    contexts: &BTreeMap<String, QueryContext>,
    fact_limit: usize,
    required: &[CompilationFactHit],
) -> Result<AggregateQueryContext, ClewError> {
    let first = contexts
        .values()
        .next()
        .ok_or_else(|| invalid("compilation query set is empty"))?;
    if contexts
        .values()
        .any(|context| context.requested_terms != first.requested_terms)
    {
        return Err(invalid("compilation queries have different term authority"));
    }
    let all_facts = contexts
        .iter()
        .flat_map(|(compilation, context)| {
            context
                .facts
                .iter()
                .cloned()
                .map(|fact| CompilationFactHit {
                    compilation: compilation.clone(),
                    fact,
                })
        })
        .collect::<BTreeSet<_>>();
    let lanes = contexts
        .iter()
        .map(|(compilation, context)| {
            context
                .facts
                .iter()
                .cloned()
                .map(|fact| CompilationFactHit {
                    compilation: compilation.clone(),
                    fact,
                })
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let mut facts = fair_compilation_selection(&lanes, fact_limit);
    if !required.is_empty() {
        if required.len() > fact_limit
            || required.iter().collect::<BTreeSet<_>>().len() != required.len()
            || required.iter().any(|fact| !all_facts.contains(fact))
        {
            return Err(invalid(
                "required exact declarations are inconsistent with query authority",
            ));
        }
        facts.retain(|fact| !required.contains(fact));
        while facts.len().saturating_add(required.len()) > fact_limit {
            facts.pop();
        }
        facts.splice(0..0, required.iter().cloned());
    }
    let truncated =
        contexts.values().any(|context| context.truncated) || all_facts.len() > facts.len();
    let unmatched_terms = first
        .requested_terms
        .iter()
        .filter(|term| {
            contexts
                .values()
                .all(|context| context.unmatched_terms.contains(term))
        })
        .cloned()
        .collect();
    let query_shards_read = contexts.values().try_fold(0u32, |total, context| {
        total
            .checked_add(context.query_shards_read)
            .ok_or_else(|| resource("aggregate query shard count overflow"))
    })?;
    Ok(AggregateQueryContext {
        schema: AGGREGATE_QUERY_CONTEXT_SCHEMA.into(),
        index_id: canonical::hash(
            &contexts
                .iter()
                .map(|(compilation, context)| (compilation, &context.index_id))
                .collect::<BTreeMap<_, _>>(),
        )
        .map_err(internal)?,
        requested_terms: first.requested_terms.clone(),
        unmatched_terms,
        facts,
        query_shards_read,
        truncated,
    })
}

fn fair_compilation_selection(
    facts_by_compilation: &[Vec<CompilationFactHit>],
    limit: usize,
) -> Vec<CompilationFactHit> {
    let mut selected = BTreeSet::new();
    let mut cursors = vec![0usize; facts_by_compilation.len()];
    while selected.len() < limit {
        let mut progressed = false;
        for (facts, cursor) in facts_by_compilation.iter().zip(&mut cursors) {
            while *cursor < facts.len() {
                let fact = facts[*cursor].clone();
                *cursor += 1;
                if selected.insert(fact) {
                    progressed = true;
                    break;
                }
            }
            if selected.len() == limit {
                break;
            }
        }
        if !progressed {
            break;
        }
    }
    selected.into_iter().collect()
}

fn promote_query_fact(context: &mut QueryContext, selected: &FactHit, limit: usize) {
    let original_len = context.facts.len();
    let already_selected = context.facts.iter().any(|fact| fact == selected);
    context.facts.retain(|fact| fact != selected);
    while context.facts.len() >= limit {
        context.facts.pop();
    }
    context.facts.insert(0, selected.clone());
    context.truncated |=
        original_len > context.facts.len() || (!already_selected && original_len >= limit);
}

fn select_bounded_exact_expansion_matches(
    matches: &mut BTreeMap<String, BTreeSet<CompilationFactHit>>,
    terms: &[String],
    per_term_limit: usize,
) -> (Vec<CompilationFactHit>, bool) {
    let mut selected = Vec::new();
    let mut truncated = false;
    for term in terms {
        let term_matches = matches.remove(term).unwrap_or_default();
        truncated |= term_matches.len() > per_term_limit;
        selected.extend(term_matches.into_iter().take(per_term_limit));
    }
    (selected, truncated)
}

fn select_unique_exact_match(
    matches: BTreeSet<CompilationFactHit>,
    direct_posting_truncated: bool,
) -> Result<CompilationFactHit, ClewError> {
    if direct_posting_truncated {
        return Err(ClewError::new(
            ErrorCode::IncompleteSemanticAnalysis,
            "exact declaration-name posting is truncated; selection cannot prove uniqueness",
        ));
    }
    match matches.len() {
        0 => Err(ClewError::new(
            ErrorCode::SymbolNotFound,
            "no exact declaration matches the selected file and term",
        )),
        1 => Ok(matches.into_iter().next().expect("one exact match")),
        _ => Err(ClewError::new(
            ErrorCode::AmbiguousSymbol,
            "multiple exact declarations match the selected file and term",
        )),
    }
}

fn load_fact_evidence(
    store: &CasStore,
    facts: &[CompilationFactHit],
) -> Result<Vec<Value>, ClewError> {
    facts
        .iter()
        .map(|hit| {
            let fact = &hit.fact;
            let limit = usize::try_from(fact.payload.size)
                .map_err(|_| resource("fact payload exceeds host size"))?;
            if limit > MAX_PAYLOAD_BYTES {
                return Ok(json!({
                    "compilation":hit.compilation,
                    "factKey":fact.fact_key,
                    "domainUri":fact.domain_uri,
                    "payloadRef":fact.payload,
                    "payload":{
                        "opaquePayloadDigest":fact.payload.digest,
                        "size":fact.payload.size,
                        "reason":"PAYLOAD_EXCEEDS_CONTEXT_LIMIT",
                    },
                }));
            }
            let lease = store.read(&fact.payload, limit)?;
            let payload = serde_json::from_slice::<Value>(lease.bytes()).unwrap_or_else(
                |_| json!({"opaquePayloadDigest":fact.payload.digest,"size":fact.payload.size}),
            );
            Ok(json!({
                "compilation":hit.compilation,
                "factKey":fact.fact_key,
                "domainUri":fact.domain_uri,
                "payloadRef":fact.payload,
                "payload":payload,
            }))
        })
        .collect()
}

fn load_fact_payload(store: &CasStore, fact: &FactHit) -> Result<Value, ClewError> {
    let limit = usize::try_from(fact.payload.size)
        .map_err(|_| resource("fact payload exceeds host size"))?;
    if limit > MAX_PAYLOAD_BYTES {
        return Err(resource(
            "exact declaration payload exceeds the context payload limit",
        ));
    }
    let lease = store.read(&fact.payload, limit)?;
    serde_json::from_slice(lease.bytes())
        .map_err(|_| invalid("exact declaration payload is not JSON"))
}

fn paths_in_payload(payload: &Value) -> Vec<String> {
    let mut paths = BTreeSet::new();
    collect_paths(payload, None, &mut paths, 0);
    paths.into_iter().collect()
}

/// Return the normalized repository-relative source identities carried by one
/// selected fact payload.  Thread composition uses this narrow seam to retain
/// exact compilation provenance for source windows without exposing the
/// context implementation's ranking or snapshot internals.
pub(crate) fn fact_source_paths(payload: &Value) -> Vec<String> {
    paths_in_payload(payload)
}

fn ordered_paths_in_evidence(facts: &[Value]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut paths = Vec::new();
    for fact in facts {
        for path in paths_in_payload(&fact["payload"]) {
            if seen.insert(path.clone()) {
                paths.push(path);
            }
        }
    }
    paths
}

fn collect_paths(value: &Value, key: Option<&str>, output: &mut BTreeSet<String>, depth: usize) {
    if depth > 32 {
        return;
    }
    match value {
        Value::String(value)
            if key.is_some_and(|key| {
                matches!(
                    key,
                    "path" | "file" | "fileId" | "relativePath" | "sourcePath"
                )
            }) && safe_path(value) =>
        {
            output.insert(value.clone());
        }
        Value::Array(values) => {
            for value in values {
                collect_paths(value, key, output, depth + 1);
            }
        }
        Value::Object(values) => {
            for (key, value) in values {
                collect_paths(value, Some(key), output, depth + 1);
            }
        }
        _ => {}
    }
}

fn load_source_snippets(
    store: &CasStore,
    snapshot: &RepositoryInputSnapshot,
    paths: &[String],
    terms: &[String],
    source_hints: &SourceRangeHints,
    complete_small_sources: bool,
) -> Result<Vec<Value>, ClewError> {
    let worktree = snapshot
        .worktree
        .iter()
        .filter_map(|entry| {
            entry
                .content
                .as_ref()
                .map(|content| (entry.path.as_str(), (entry.kind, content)))
        })
        .collect::<BTreeMap<_, _>>();
    let index = snapshot
        .index
        .iter()
        .filter(|entry| entry.stage == 0)
        .map(|entry| (entry.path.as_str(), &entry.content))
        .collect::<BTreeMap<_, _>>();
    let mut snippets = Vec::new();
    for path in paths {
        let content = match worktree.get(path.as_str()) {
            Some((WorktreeKind::Regular, content)) => Some(*content),
            Some((WorktreeKind::Missing, _)) => None,
            Some((WorktreeKind::Symlink, _)) => None,
            None => index.get(path.as_str()).copied(),
        };
        let Some(content) = content else { continue };
        let limit =
            usize::try_from(content.size).map_err(|_| resource("source exceeds host size"))?;
        let lease = store.read(content, limit)?;
        let Ok(source) = std::str::from_utf8(lease.bytes()) else {
            continue;
        };
        let windows = source_windows_with_policy(
            source,
            terms,
            source_hints.get(path),
            complete_small_sources,
        );
        let start_line = windows.first().map(|window| window.0).unwrap_or(1);
        let end_line = windows.last().map(|window| window.1).unwrap_or(start_line);
        let text = windows
            .iter()
            .enumerate()
            .map(|(index, window)| {
                if index == 0 {
                    window.2.clone()
                } else {
                    format!("CODECLEW_OMITTED_LINES_BEFORE_{}\n{}", window.0, window.2)
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        let projected_windows = windows
            .iter()
            .map(|(start_line, end_line, text)| {
                json!({
                    "startLine":start_line,
                    "endLine":end_line,
                    "text":text,
                })
            })
            .collect::<Vec<_>>();
        snippets.push(json!({
            "fileId":path,
            "contentRef":content,
            "startLine":start_line,
            "endLine":end_line,
            "text":text,
            "windows":projected_windows,
            "completeFile":windows.len() == 1 && windows[0].2.len() == source.len(),
        }));
    }
    Ok(snippets)
}

type SourceRangeHints = BTreeMap<String, BTreeMap<usize, Option<usize>>>;

fn source_offset_hints(facts: &[Value], terms: &[String]) -> SourceRangeHints {
    let lowered_terms = terms
        .iter()
        .map(|term| term.to_lowercase())
        .collect::<BTreeSet<_>>();
    let mut hints = BTreeMap::new();
    for fact in facts {
        collect_source_offset_hints(&fact["payload"], &lowered_terms, &mut hints, 0);
    }
    hints
}

fn collect_source_offset_hints(
    value: &Value,
    terms: &BTreeSet<String>,
    output: &mut SourceRangeHints,
    depth: usize,
) {
    if depth > 32 {
        return;
    }
    match value {
        Value::Array(values) => {
            for value in values {
                collect_source_offset_hints(value, terms, output, depth + 1);
            }
        }
        Value::Object(values) => {
            let exact_name_match = values
                .get("name")
                .and_then(Value::as_str)
                .is_some_and(|name| terms.contains(&name.to_lowercase()));
            let identity_terms = exact_identity_terms(value);
            let exact_identity_match = terms.iter().any(|term| identity_terms.contains(term));
            if exact_name_match || exact_identity_match {
                let file = values
                    .get("sourceOrigin")
                    .and_then(Value::as_object)
                    .and_then(|origin| origin.get("file"))
                    .and_then(Value::as_str)
                    .or_else(|| values.get("file").and_then(Value::as_str));
                let offset = values
                    .get("sourceOrigin")
                    .and_then(Value::as_object)
                    .and_then(|origin| origin.get("rangeStart"))
                    .and_then(Value::as_u64)
                    .or_else(|| values.get("rangeStart").and_then(Value::as_u64))
                    .or_else(|| values.get("start").and_then(Value::as_u64));
                let end = values
                    .get("sourceOrigin")
                    .and_then(Value::as_object)
                    .and_then(|origin| origin.get("rangeEnd"))
                    .and_then(Value::as_u64)
                    .or_else(|| values.get("rangeEnd").and_then(Value::as_u64))
                    .or_else(|| values.get("end").and_then(Value::as_u64));
                if let (Some(file), Some(offset)) = (file, offset)
                    && safe_path(file)
                    && let Ok(offset) = usize::try_from(offset)
                {
                    let end = end
                        .and_then(|end| usize::try_from(end).ok())
                        .filter(|end| *end > offset);
                    let ranges = output.entry(file.to_owned()).or_default();
                    if let Some(observed) = ranges.get_mut(&offset) {
                        if let Some(end) = end {
                            *observed = Some(observed.map_or(end, |current| current.max(end)));
                        }
                    } else if ranges.len() < MAX_SOURCE_WINDOWS {
                        // Facts arrive in deterministic task-relevance and
                        // fairness order. Preserve that authority before the
                        // range map sorts by byte offset, otherwise four early
                        // broad matches can evict an exact
                        // declaration requested later in a large source file.
                        ranges.insert(offset, end);
                    }
                }
            }
            for value in values.values() {
                collect_source_offset_hints(value, terms, output, depth + 1);
            }
        }
        _ => {}
    }
}

fn declaration_window(
    source: &str,
    start: Option<usize>,
    end: Option<usize>,
) -> Option<(usize, usize, String)> {
    let (start, end) = (start?, end?);
    if start >= end
        || end > source.len()
        || !source.is_char_boundary(start)
        || !source.is_char_boundary(end)
    {
        return None;
    }
    let line_start = source[..start]
        .rfind('\n')
        .map(|index| index + 1)
        .unwrap_or(0);
    let line_end = source[end..]
        .find('\n')
        .map(|index| end + index)
        .unwrap_or(source.len());
    if line_end.saturating_sub(line_start) > MAX_SOURCE_BYTES {
        return None;
    }
    let text = source[line_start..line_end].to_owned();
    let start_line = source[..line_start]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1;
    let end_line = start_line + text.lines().count().max(1) - 1;
    Some((start_line, end_line, text))
}

#[cfg(test)]
fn source_windows(
    source: &str,
    terms: &[String],
    source_ranges: Option<&BTreeMap<usize, Option<usize>>>,
) -> Vec<(usize, usize, String)> {
    source_windows_with_policy(source, terms, source_ranges, false)
}

fn source_windows_with_policy(
    source: &str,
    terms: &[String],
    source_ranges: Option<&BTreeMap<usize, Option<usize>>>,
    complete_small_source: bool,
) -> Vec<(usize, usize, String)> {
    if complete_small_source && !source.is_empty() && source.len() <= MAX_COMPLETE_FILE_SOURCE_BYTES
    {
        return vec![(1, source.lines().count().max(1), source.to_owned())];
    }
    let has_declaration_ranges = source_ranges.is_some_and(|ranges| !ranges.is_empty());
    let ranges = source_ranges
        .filter(|ranges| !ranges.is_empty())
        .map(|ranges| {
            ranges
                .iter()
                .map(|(start, end)| (Some(*start), *end))
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| vec![(None, None)]);
    let mut windows = Vec::new();
    let mut projected_bytes = 0usize;
    let line_ranges = source_line_ranges(source);
    let lines = line_ranges
        .iter()
        .filter_map(|(start, end)| source.get(*start..*end))
        .collect::<Vec<_>>();
    for (offset, end) in ranges {
        let window = declaration_window(source, offset, end)
            .map(|window| {
                declaration_context_window(
                    source,
                    &line_ranges,
                    window,
                    SOURCE_DECLARATION_CONTEXT_BEFORE_LINES,
                    SOURCE_DECLARATION_CONTEXT_AFTER_LINES,
                )
            })
            .unwrap_or_else(|| snippet(source, terms, offset, &lines, &line_ranges));
        if windows.iter().any(|existing: &(usize, usize, String)| {
            existing.0 == window.0 && existing.1 == window.1
        }) {
            continue;
        }
        if let Some(previous) = windows.last_mut()
            && window.0 <= previous.1.saturating_add(1)
        {
            let merged_end = previous.1.max(window.1);
            let Some(merged_text) = exact_line_window(source, &line_ranges, previous.0, merged_end)
            else {
                break;
            };
            let merged_bytes = projected_bytes
                .saturating_sub(previous.2.len())
                .saturating_add(merged_text.len());
            if merged_bytes > MAX_SOURCE_BYTES {
                break;
            }
            previous.1 = merged_end;
            previous.2 = merged_text;
            projected_bytes = merged_bytes;
            continue;
        }
        if windows.len() == MAX_SOURCE_WINDOWS
            || (!windows.is_empty()
                && projected_bytes.saturating_add(window.2.len()) > MAX_SOURCE_BYTES)
        {
            break;
        }
        projected_bytes = projected_bytes.saturating_add(window.2.len());
        windows.push(window);
    }
    if has_declaration_ranges {
        let lowered_terms = terms
            .iter()
            .map(|term| term.to_lowercase())
            .collect::<Vec<_>>();
        while windows.len() < MAX_SOURCE_WINDOWS {
            let (best_support, _) = best_lexical_support(&lines, &windows, &lowered_terms);
            let Some((_, line, text)) = best_support else {
                break;
            };
            let exact_text = exact_line_window(source, &line_ranges, line, line)
                .unwrap_or_else(|| text.to_owned());
            if projected_bytes.saturating_add(exact_text.len()) > MAX_SOURCE_BYTES {
                break;
            }
            projected_bytes = projected_bytes.saturating_add(exact_text.len());
            windows.push((line, line, exact_text));
            windows.sort_by_key(|window| (window.0, window.1));
        }
    }
    windows
}

fn declaration_context_window(
    source: &str,
    line_ranges: &[(usize, usize)],
    exact: (usize, usize, String),
    before: usize,
    after: usize,
) -> (usize, usize, String) {
    let exact_line_count = exact.1.saturating_sub(exact.0).saturating_add(1);
    if exact_line_count > MAX_CONTEXTUAL_DECLARATION_LINES {
        return exact;
    }
    let start = exact.0.saturating_sub(1).saturating_sub(before);
    let end = exact.1.saturating_add(after).min(line_ranges.len());
    if start >= end {
        return exact;
    }
    let Some(text) = exact_line_window(source, line_ranges, start + 1, end) else {
        return exact;
    };
    if text.len() > MAX_SNIPPET_BYTES {
        exact
    } else {
        (start + 1, end, text)
    }
}

fn source_line_ranges(source: &str) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut start = 0usize;
    for (index, byte) in source.bytes().enumerate() {
        if byte == b'\n' {
            ranges.push((start, index));
            start = index + 1;
        }
    }
    if start < source.len() {
        ranges.push((start, source.len()));
    }
    if ranges.is_empty() {
        ranges.push((0, 0));
    }
    ranges
}

fn exact_line_window(
    source: &str,
    line_ranges: &[(usize, usize)],
    start_line: usize,
    end_line: usize,
) -> Option<String> {
    if start_line == 0 || start_line > end_line || end_line > line_ranges.len() {
        return None;
    }
    let start = line_ranges.get(start_line - 1)?.0;
    let end = line_ranges.get(end_line - 1)?.1;
    source.get(start..end).map(str::to_owned)
}

type LexicalSupport<'a> = ((usize, usize), usize, &'a str);

fn best_lexical_support<'a>(
    lines: &[&'a str],
    windows: &[(usize, usize, String)],
    lowered_terms: &[String],
) -> (Option<LexicalSupport<'a>>, usize) {
    let retained_lines = windows
        .iter()
        .flat_map(|window| window.2.lines())
        .map(str::to_lowercase)
        .collect::<Vec<_>>();
    let mut retained_scores = BTreeMap::<Vec<usize>, (usize, usize)>::new();
    let best = lines
        .iter()
        .enumerate()
        .filter(|(index, _)| {
            let line = index + 1;
            !windows
                .iter()
                .any(|window| window.0 <= line && line <= window.1)
        })
        .filter_map(|(index, line)| {
            let lowered_line = line.to_lowercase();
            let support_terms = lowered_terms
                .iter()
                .enumerate()
                .filter_map(|(term_index, term)| {
                    lowered_line.contains(term.as_str()).then_some(term_index)
                })
                .collect::<Vec<_>>();
            if support_terms.is_empty() {
                return None;
            }
            let score = lexical_subset_score(&lowered_line, lowered_terms, &support_terms);
            let retained_score = match retained_scores.get(&support_terms).copied() {
                Some(score) => score,
                None if retained_scores.len() == MAX_SUPPORT_TERM_SETS => return None,
                None => {
                    let score = retained_lines
                        .iter()
                        .map(|retained| {
                            lexical_subset_score(retained, lowered_terms, &support_terms)
                        })
                        .max()
                        .unwrap_or((0, 0));
                    retained_scores.insert(support_terms, score);
                    score
                }
            };
            (score > retained_score).then_some((score, index + 1, *line))
        })
        .max_by(|left, right| left.0.cmp(&right.0).then_with(|| right.1.cmp(&left.1)));
    (best, retained_scores.len())
}

fn lexical_subset_score(
    lowered_line: &str,
    lowered_terms: &[String],
    term_indexes: &[usize],
) -> (usize, usize) {
    let mut coverage = 0usize;
    let mut occurrences = 0usize;
    for term_index in term_indexes {
        let count = lowered_line.matches(&lowered_terms[*term_index]).count();
        coverage += usize::from(count > 0);
        occurrences = occurrences.saturating_add(count);
    }
    (coverage, occurrences)
}

fn snippet(
    source: &str,
    terms: &[String],
    source_offset: Option<usize>,
    lines: &[&str],
    line_ranges: &[(usize, usize)],
) -> (usize, usize, String) {
    if source.len() <= MAX_SNIPPET_BYTES {
        return (1, source.lines().count().max(1), source.to_owned());
    }
    let lowered_terms = terms
        .iter()
        .map(|term| term.to_lowercase())
        .collect::<Vec<_>>();
    let hit = source_offset
        .map(|offset| {
            let offset = offset.min(source.len());
            source
                .char_indices()
                .take_while(|(index, _)| *index < offset)
                .filter(|(_, character)| *character == '\n')
                .count()
        })
        .or_else(|| {
            let mut best = None;
            let mut best_score = 0usize;
            for (index, line) in lines.iter().enumerate() {
                let line = line.to_lowercase();
                let score = lowered_terms
                    .iter()
                    .filter(|term| line.contains(term.as_str()))
                    .count();
                if score > best_score {
                    best = Some(index);
                    best_score = score;
                }
            }
            best
        })
        .unwrap_or(0);
    let start = hit.saturating_sub(20);
    let mut end = (hit + 40).min(lines.len());
    let mut text = exact_line_window(source, line_ranges, start + 1, end).unwrap_or_default();
    while text.len() > MAX_SNIPPET_BYTES && end > start + 1 {
        end -= 1;
        text = exact_line_window(source, line_ranges, start + 1, end).unwrap_or_default();
    }
    (start + 1, end, text)
}

fn retained_project_compiler_versions(
    store: &CasStore,
    language: crate::session::SessionLanguage,
    compilations: &[crate::generation_service::ReadyGeneration],
) -> Result<BTreeMap<String, String>, ClewError> {
    let mut versions = BTreeMap::new();
    for compilation in compilations {
        let version = if language == crate::session::SessionLanguage::Kotlin {
            let lease = store.read(&compilation.derived_input_manifest, MAX_PAYLOAD_BYTES)?;
            let derived: crate::derived_manifest::DerivedAnalysisInputManifest =
                serde_json::from_slice(lease.bytes())
                    .map_err(|_| invalid("retained project input authority is invalid"))?;
            if derived.repository_snapshot != compilation.repository_snapshot {
                return Err(invalid("retained project snapshot differs from generation"));
            }
            let provider = derived
                .provider_models
                .iter()
                .find(|provider| provider.build_model.provider_id == "project-native-kotlin")
                .ok_or_else(|| invalid("retained Kotlin project model is missing"))?;
            let lease = store.read(&provider.build_model.model, MAX_PAYLOAD_BYTES)?;
            let model: Value = serde_json::from_slice(lease.bytes())
                .map_err(|_| invalid("retained Kotlin project model is invalid"))?;
            retained_kotlin_compiler_version(&model, &compilation.compilation)?
        } else {
            compilation.compiler_version.clone()
        };
        versions.insert(compilation.compilation.clone(), version);
    }
    Ok(versions)
}

fn retained_kotlin_compiler_version(model: &Value, compilation: &str) -> Result<String, ClewError> {
    if model["schema"] != "codeclew-project-native-model/2.0" || model["compilation"] != compilation
    {
        return Err(invalid(
            "retained Kotlin model compilation authority differs",
        ));
    }
    model
        .pointer("/projectSemantics/projectCompilerVersion")
        .and_then(Value::as_str)
        .filter(|version| !version.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| invalid("retained project Kotlin compiler authority is missing"))
}

fn bounded_projection(context: &Value) -> Result<Value, ClewError> {
    let mut projection = json!({
        "schema":BOUNDED_CONTEXT_PROJECTION_SCHEMA,
        "language":context["language"],
        "snapshot":context["snapshot"],
        "task":context["task"],
        "compilations":context["compilations"],
        "compilerVersions":context["compilerVersions"],
        "generationAuthority":context["generationAuthority"],
        "matches":[],
        "sources":[],
        "completeness":context["completeness"],
        "verificationObligations":context["verificationObligations"],
        "truncated":false,
    });
    if let Some(versions) = context.get("projectCompilerVersions") {
        projection["projectCompilerVersions"] = versions.clone();
    }
    let mut projected_bytes = canonical::bytes(&projection).map_err(internal)?.len();
    // Structured semantic facts are the primary navigation authority. Large
    // source snippets are useful supporting evidence, but must not consume the
    // bounded projection before any fact that identifies why the source was
    // selected can be returned.
    for key in ["matches", "sources"] {
        for value in context[key].as_array().into_iter().flatten() {
            if key == "matches"
                && value.pointer("/payload/kind").and_then(Value::as_str) == Some("source-file")
            {
                projection["truncated"] = Value::Bool(true);
                continue;
            }
            let item_bytes = canonical::bytes(value).map_err(internal)?.len();
            let separator_bytes =
                usize::from(!projection[key].as_array().expect("known array").is_empty());
            let candidate_bytes = projected_bytes
                .checked_add(item_bytes)
                .and_then(|size| size.checked_add(separator_bytes))
                .ok_or_else(|| resource("bounded projection size overflow"))?;
            if candidate_bytes > PROJECTION_TARGET_BYTES {
                projection["truncated"] = Value::Bool(true);
                continue;
            }
            projection[key]
                .as_array_mut()
                .expect("known array")
                .push(value.clone());
            projected_bytes = candidate_bytes;
        }
    }
    if canonical::bytes(&projection).map_err(internal)?.len() > PROJECTION_TARGET_BYTES {
        return Err(resource("bounded projection exceeds its output limit"));
    }
    Ok(projection)
}

fn rank_fact_evidence(
    facts: Vec<Value>,
    terms: &[String],
    limit: usize,
) -> Result<Vec<Value>, ClewError> {
    let lowered = terms
        .iter()
        .map(|term| term.to_lowercase())
        .collect::<Vec<_>>();
    let direct_names = lowered
        .iter()
        .map(|term| normalized_direct_name(term))
        .collect::<BTreeSet<_>>();
    let mut lanes = BTreeMap::<String, Vec<Value>>::new();
    for fact in facts {
        let compilation = fact
            .get("compilation")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("ranked context fact has no compilation provenance"))?;
        lanes.entry(compilation.to_owned()).or_default().push(fact);
    }
    for facts in lanes.values_mut() {
        let mut decorated = std::mem::take(facts)
            .into_iter()
            .map(|fact| {
                let is_declaration = fact["payload"]
                    .get("kind")
                    .and_then(Value::as_str)
                    .is_some_and(|kind| kind.eq_ignore_ascii_case("declaration"))
                    || (fact["payload"].get("declarationKind").is_some()
                        && fact["payload"].get("symbolIdentity").is_some());
                let direct_name_coverage = if is_declaration {
                    fact["payload"]
                        .get("name")
                        .and_then(Value::as_str)
                        .map(|name| {
                            usize::from(direct_names.contains(&normalized_direct_name(name)))
                        })
                        .unwrap_or(0)
                } else {
                    0
                };
                let identities = exact_identity_terms(&fact["payload"]);
                let exact_coverage = lowered
                    .iter()
                    .filter(|term| identities.contains(term.as_str()))
                    .count();
                let score = fact_score(&fact["payload"], &lowered, None, 0);
                (
                    exact_coverage,
                    direct_name_coverage,
                    usize::from(is_declaration),
                    score,
                    fact,
                )
            })
            .collect::<Vec<_>>();
        decorated.sort_by(|left, right| {
            right
                .0
                .cmp(&left.0)
                .then_with(|| right.1.cmp(&left.1))
                .then_with(|| right.2.cmp(&left.2))
                .then_with(|| right.3.cmp(&left.3))
                .then_with(|| left.4["factKey"].as_str().cmp(&right.4["factKey"].as_str()))
        });
        *facts = decorated
            .into_iter()
            .map(|(_, _, _, _, fact)| fact)
            .collect();
    }
    let lanes = lanes.into_values().collect::<Vec<_>>();
    let mut ranked = Vec::new();
    let mut cursors = vec![0usize; lanes.len()];
    let limit = limit.max(1);
    while ranked.len() < limit {
        let mut progressed = false;
        for (facts, cursor) in lanes.iter().zip(&mut cursors) {
            if let Some(fact) = facts.get(*cursor) {
                ranked.push(fact.clone());
                *cursor += 1;
                progressed = true;
            }
            if ranked.len() == limit {
                break;
            }
        }
        if !progressed {
            break;
        }
    }
    Ok(ranked)
}

fn rank_fact_evidence_with_required(
    mut facts: Vec<Value>,
    terms: &[String],
    limit: usize,
    required: &[CompilationFactHit],
) -> Result<Vec<Value>, ClewError> {
    if required.is_empty() || required.len() > limit {
        return Err(internal("exact declaration evidence limit is invalid"));
    }
    let mut selected = Vec::with_capacity(required.len());
    for required in required {
        let position = facts
            .iter()
            .position(|fact| {
                fact.get("compilation").and_then(Value::as_str)
                    == Some(required.compilation.as_str())
                    && fact.get("factKey").and_then(Value::as_str)
                        == Some(required.fact.fact_key.as_str())
            })
            .ok_or_else(|| {
                internal("exact declaration is absent from aggregate query authority")
            })?;
        selected.push(facts.remove(position));
    }
    let remaining = limit - required.len();
    let mut ranked = if remaining > 0 {
        rank_fact_evidence(facts, terms, remaining)?
    } else {
        Vec::new()
    };
    selected.append(&mut ranked);
    let ranked = selected;
    Ok(ranked)
}

fn normalized_direct_name(value: &str) -> String {
    value
        .chars()
        .flat_map(char::to_lowercase)
        .filter(|character| *character != '_')
        .collect()
}

fn fact_score(value: &Value, terms: &[String], key: Option<&str>, depth: usize) -> usize {
    if depth > 32 {
        return 0;
    }
    match value {
        Value::String(value) => {
            let value = value.to_lowercase();
            let weight: usize = match key {
                Some(
                    "symbolIdentity" | "compilerCallableId" | "compilerClassId" | "ownerIdentity",
                ) => 16,
                Some("path" | "file" | "fileId" | "relativePath" | "sourcePath") => 8,
                Some("name" | "declarationKind") => 6,
                Some("sourceSet" | "module" | "provider" | "schema") => 0,
                _ => 1,
            };
            weight.saturating_mul(
                terms
                    .iter()
                    .filter(|term| value.contains(term.as_str()))
                    .count(),
            )
        }
        Value::Array(values) => values.iter().fold(0usize, |total, value| {
            total.saturating_add(fact_score(value, terms, key, depth + 1))
        }),
        Value::Object(values) => values.iter().fold(0usize, |total, (key, value)| {
            total.saturating_add(fact_score(value, terms, Some(key), depth + 1))
        }),
        _ => 0,
    }
}

fn exact_identity_terms(value: &Value) -> BTreeSet<String> {
    let mut output = BTreeSet::new();
    collect_exact_identity_terms(value, None, 0, &mut output);
    output
}

fn collect_exact_identity_terms(
    value: &Value,
    key: Option<&str>,
    depth: usize,
    output: &mut BTreeSet<String>,
) {
    if depth > 32 {
        return;
    }
    match value {
        Value::String(value)
            if key.is_some_and(|key| {
                matches!(
                    key,
                    "symbolIdentity"
                        | "compilerCallableId"
                        | "compilerClassId"
                        | "ownerIdentity"
                        | "identifierTerms"
                )
            }) =>
        {
            for component in value
                .split(|character: char| {
                    matches!(character, '/' | '.' | '#' | ':' | '(' | ')' | ';' | '@')
                })
                .filter(|component| !component.is_empty())
            {
                insert_identity_component_terms(component, output);
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_exact_identity_terms(value, key, depth + 1, output);
            }
        }
        Value::Object(values) => {
            for (key, value) in values {
                collect_exact_identity_terms(value, Some(key), depth + 1, output);
            }
        }
        _ => {}
    }
}

fn insert_identity_component_terms(component: &str, output: &mut BTreeSet<String>) {
    output.insert(component.chars().flat_map(char::to_lowercase).collect());
    for segment in component.split('_').filter(|segment| !segment.is_empty()) {
        insert_camel_component_terms(segment, output);
    }
}

fn insert_camel_component_terms(component: &str, output: &mut BTreeSet<String>) {
    let characters = component.chars().collect::<Vec<_>>();
    let mut start = 0usize;
    for index in 1..characters.len() {
        let previous = characters[index - 1];
        let current = characters[index];
        let next = characters.get(index + 1).copied();
        if current.is_uppercase()
            && (previous.is_lowercase()
                || previous.is_numeric()
                || (previous.is_uppercase() && next.is_some_and(char::is_lowercase)))
        {
            if index > start {
                output.insert(
                    characters[start..index]
                        .iter()
                        .copied()
                        .flat_map(char::to_lowercase)
                        .collect(),
                );
            }
            start = index;
        }
    }
    if start < characters.len() {
        output.insert(
            characters[start..]
                .iter()
                .copied()
                .flat_map(char::to_lowercase)
                .collect(),
        );
    }
}

#[cfg(test)]
fn exact_identity_match(value: &Value, term: &str, key: Option<&str>, depth: usize) -> bool {
    if depth > 32 {
        return false;
    }
    match value {
        Value::String(value)
            if key.is_some_and(|key| {
                matches!(
                    key,
                    "symbolIdentity" | "compilerCallableId" | "compilerClassId" | "ownerIdentity"
                )
            }) =>
        {
            value
                .split(|character: char| {
                    matches!(character, '/' | '.' | '#' | ':' | '(' | ')' | ';' | '@')
                })
                .any(|component| {
                    component
                        .chars()
                        .flat_map(char::to_lowercase)
                        .eq(term.chars())
                })
        }
        Value::Array(values) => values
            .iter()
            .any(|value| exact_identity_match(value, term, key, depth + 1)),
        Value::Object(values) => values
            .iter()
            .any(|(key, value)| exact_identity_match(value, term, Some(key), depth + 1)),
        _ => false,
    }
}

fn safe_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\0')
        && !path
            .split('/')
            .any(|component| component.is_empty() || component == "." || component == "..")
}

fn invalid(message: &str) -> ClewError {
    ClewError::new(ErrorCode::InvalidInput, message)
}

fn resource(message: &str) -> ClewError {
    ClewError::new(ErrorCode::ResourceLimit, message)
}

fn parse_error(error: impl std::fmt::Display) -> ClewError {
    invalid(&error.to_string())
}

fn internal(error: impl std::fmt::Display) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::{
        AGGREGATE_QUERY_CONTEXT_SCHEMA, CompilationFactHit, EXACT_EXPANSION_SELECTION_SCHEMA,
        EXACT_SELECTION_SCHEMA, ExactSelectionAuthority, MAX_PAYLOAD_BYTES, MAX_SOURCE_BYTES,
        PROJECTION_TARGET_BYTES, best_lexical_support, bounded_projection,
        exact_declaration_file_term_matches, exact_declaration_term_matches,
        exact_expansion_selection_authorities, exact_expansion_selection_hit,
        exact_selection_authorities, load_fact_evidence, merge_query_contexts,
        merge_query_contexts_with_required, ordered_paths_in_evidence, promote_query_fact,
        rank_fact_evidence, rank_fact_evidence_with_required,
        select_bounded_exact_expansion_matches, select_unique_exact_match, source_offset_hints,
        source_windows, source_windows_with_policy, validate_exact_selection, validate_source_rows,
    };
    use crate::adapter_v2::CapabilityUri;
    use crate::cas::CasStore;
    use crate::error::ErrorCode;
    use crate::query_v2::{FactHit, QUERY_CONTEXT_SCHEMA, QueryContext};
    use crate::state::StateAuthority;
    use serde_json::json;
    use std::collections::{BTreeMap, BTreeSet};

    #[test]
    fn kotlin_exact_file_selection_matches_identity_and_keeps_overload_ambiguity() {
        let symbol = "callable:sample/Receiver.accept#jvm:(Ljava/util/List;)V";
        let payload = json!({
            "declarationKind":"FUNCTION", "symbolIdentity":symbol,
            "compilerCallableId":"sample/Receiver.accept", "ownerIdentity":"class:sample/Receiver",
            "file":"Receiver.kt"
        });
        assert!(exact_declaration_file_term_matches(
            &payload,
            "Receiver.kt",
            symbol
        ));
        assert!(exact_declaration_file_term_matches(
            &payload,
            "Receiver.kt",
            "accept"
        ));
        assert!(!exact_declaration_file_term_matches(
            &payload, "Other.kt", symbol
        ));
        assert!(!exact_declaration_term_matches(
            &payload,
            "class:sample/Receiver"
        ));
        let hit = |name: &str| CompilationFactHit {
            compilation: ":/main".into(),
            fact: FactHit {
                fact_key: name.into(),
                domain_uri: CapabilityUri::parse("analysis:symbol").unwrap(),
                payload: crate::cas::CasObject::for_bytes("test/payload/1", name.as_bytes())
                    .unwrap(),
            },
        };
        assert_eq!(
            select_unique_exact_match(BTreeSet::from([hit("first"), hit("second")]), false)
                .unwrap_err()
                .code,
            ErrorCode::AmbiguousSymbol
        );
        assert!(select_unique_exact_match(BTreeSet::from([hit("first")]), false).is_ok());
    }

    #[test]
    fn required_exact_fact_survives_query_merge_and_evidence_limits() {
        let root = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(root.path().join("v2")).unwrap();
        let store = CasStore::open(&state).unwrap();
        let payload = store.put("test/payload/1", b"{}").unwrap();
        let fact = |key: &str| FactHit {
            fact_key: key.into(),
            domain_uri: CapabilityUri::parse("analysis:test").unwrap(),
            payload: payload.clone(),
        };
        let required = ["x:first-required", "y:second-required", "z:third-required"]
            .into_iter()
            .map(|key| CompilationFactHit {
                compilation: "cargo:demo".into(),
                fact: fact(key),
            })
            .collect::<Vec<_>>();
        let mut query = QueryContext {
            schema: QUERY_CONTEXT_SCHEMA.into(),
            index_id: "sha256:index".into(),
            requested_terms: vec!["value".into()],
            unmatched_terms: vec![],
            facts: vec![fact("a:first"), fact("b:second")],
            query_shards_read: 1,
            truncated: false,
        };
        for selected in required.iter().rev() {
            promote_query_fact(&mut query, &selected.fact, 5);
        }
        assert_eq!(
            &query.facts[..required.len()],
            required
                .iter()
                .map(|selected| selected.fact.clone())
                .collect::<Vec<_>>()
        );

        let aggregate = merge_query_contexts_with_required(
            &BTreeMap::from([("cargo:demo".into(), query)]),
            3,
            &required,
        )
        .unwrap();
        assert_eq!(aggregate.facts, required);

        let ranked = rank_fact_evidence_with_required(
            vec![
                json!({"compilation":"cargo:demo","factKey":"a:first","payload":{}}),
                json!({"compilation":"cargo:demo","factKey":"x:first-required","payload":{}}),
                json!({"compilation":"cargo:demo","factKey":"y:second-required","payload":{}}),
                json!({"compilation":"cargo:demo","factKey":"z:third-required","payload":{}}),
            ],
            &["value".into()],
            3,
            &aggregate.facts,
        )
        .unwrap();
        assert_eq!(ranked.len(), 3);
        assert_eq!(ranked[0]["factKey"], "x:first-required");
        assert_eq!(ranked[1]["factKey"], "y:second-required");
        assert_eq!(ranked[2]["factKey"], "z:third-required");
    }

    #[test]
    fn exact_expansion_authority_reproduces_required_aggregate_selection() {
        let root = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(root.path().join("v2")).unwrap();
        let store = CasStore::open(&state).unwrap();
        let payload = store.put("test/payload/1", b"{}").unwrap();
        let fact = |key: &str| FactHit {
            fact_key: key.into(),
            domain_uri: CapabilityUri::parse("analysis:test").unwrap(),
            payload: payload.clone(),
        };
        let query = QueryContext {
            schema: QUERY_CONTEXT_SCHEMA.into(),
            index_id: "sha256:index".into(),
            requested_terms: vec!["bytes".into(), "hash".into()],
            unmatched_terms: vec![],
            facts: vec![fact("a:other"), fact("z:bytes"), fact("y:hash")],
            query_shards_read: 1,
            truncated: false,
        };
        let queries = BTreeMap::from([("cargo:demo".into(), query)]);
        let evidence = json!({
            "exactExpansionSelections":[
                {"schema":EXACT_EXPANSION_SELECTION_SCHEMA,"term":"bytes","compilation":"cargo:demo","factKey":"z:bytes"},
                {"schema":EXACT_EXPANSION_SELECTION_SCHEMA,"term":"hash","compilation":"cargo:demo","factKey":"y:hash"}
            ]
        });
        let selections = exact_expansion_selection_authorities(&evidence).unwrap();
        let required = selections
            .iter()
            .map(|selection| exact_expansion_selection_hit(&queries, selection).unwrap())
            .collect::<Vec<_>>();
        let aggregate = merge_query_contexts_with_required(&queries, 2, &required).unwrap();
        let reproduced = merge_query_contexts_with_required(&queries, 2, &required).unwrap();
        assert_eq!(aggregate, reproduced);
        assert_eq!(aggregate.facts, required);

        let duplicated = json!({
            "exactExpansionSelections":[
                {"schema":EXACT_EXPANSION_SELECTION_SCHEMA,"term":"bytes","compilation":"cargo:demo","factKey":"z:bytes"},
                {"schema":EXACT_EXPANSION_SELECTION_SCHEMA,"term":"hash","compilation":"cargo:demo","factKey":"z:bytes"}
            ]
        });
        assert!(exact_expansion_selection_authorities(&duplicated).is_err());
    }

    #[test]
    fn exact_declaration_selection_is_structural_and_file_scoped() {
        let selected = json!({
            "kind":"declaration",
            "name":"value",
            "file":"crates/clew/src/canonical.rs"
        });
        assert!(exact_declaration_file_term_matches(
            &selected,
            "crates/clew/src/canonical.rs",
            "value"
        ));
        assert!(!exact_declaration_file_term_matches(
            &selected,
            "crates/clew/src/operations.rs",
            "value"
        ));
        assert!(!exact_declaration_file_term_matches(
            &json!({"kind":"relation","name":"value","file":"crates/clew/src/canonical.rs"}),
            "crates/clew/src/canonical.rs",
            "value"
        ));
        assert!(exact_declaration_term_matches(&selected, "value"));
    }

    #[test]
    fn bounded_exact_name_expansion_retains_each_term_and_reports_omissions() {
        let root = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(root.path().join("v2")).unwrap();
        let store = CasStore::open(&state).unwrap();
        let payload = store.put("test/payload/1", b"{}").unwrap();
        let hit = |key: &str| CompilationFactHit {
            compilation: "cargo:demo".into(),
            fact: FactHit {
                fact_key: key.into(),
                domain_uri: CapabilityUri::parse("analysis:test").unwrap(),
                payload: payload.clone(),
            },
        };
        let mut matches = BTreeMap::from([
            (
                "bytes".into(),
                (0..5).map(|index| hit(&format!("bytes-{index}"))).collect(),
            ),
            ("hash".into(), [hit("hash-0")].into_iter().collect()),
        ]);
        let (selected, truncated) = select_bounded_exact_expansion_matches(
            &mut matches,
            &["bytes".into(), "hash".into()],
            4,
        );
        assert!(truncated);
        assert_eq!(selected.len(), 5);
        assert_eq!(selected[0].fact.fact_key, "bytes-0");
        assert_eq!(selected[3].fact.fact_key, "bytes-3");
        assert_eq!(selected[4].fact.fact_key, "hash-0");
    }

    #[test]
    fn exact_declaration_selection_refuses_ambiguity_and_incomplete_postings() {
        let root = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(root.path().join("v2")).unwrap();
        let store = CasStore::open(&state).unwrap();
        let payload = store.put("test/payload/1", b"{}").unwrap();
        let hit = |compilation: &str, key: &str| CompilationFactHit {
            compilation: compilation.into(),
            fact: FactHit {
                fact_key: key.into(),
                domain_uri: CapabilityUri::parse("analysis:test").unwrap(),
                payload: payload.clone(),
            },
        };
        assert_eq!(
            select_unique_exact_match(BTreeSet::new(), false)
                .unwrap_err()
                .code,
            ErrorCode::SymbolNotFound
        );
        assert_eq!(
            select_unique_exact_match(
                [hit("cargo:a", "fact:a"), hit("cargo:b", "fact:b")]
                    .into_iter()
                    .collect(),
                false,
            )
            .unwrap_err()
            .code,
            ErrorCode::AmbiguousSymbol
        );
        assert_eq!(
            select_unique_exact_match([hit("cargo:a", "fact:a")].into_iter().collect(), true,)
                .unwrap_err()
                .code,
            ErrorCode::IncompleteSemanticAnalysis
        );
    }

    #[test]
    fn exact_selection_validation_binds_fact_payload_and_covering_source_window() {
        let root = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(root.path().join("v2")).unwrap();
        let store = CasStore::open(&state).unwrap();
        let payload_ref = store
            .put(
                "test/payload/1",
                br#"{"kind":"declaration","name":"value"}"#,
            )
            .unwrap();
        let required = CompilationFactHit {
            compilation: "cargo:demo".into(),
            fact: FactHit {
                fact_key: "fact:value".into(),
                domain_uri: CapabilityUri::parse("analysis:test").unwrap(),
                payload: payload_ref.clone(),
            },
        };
        let selection = ExactSelectionAuthority {
            schema: EXACT_SELECTION_SCHEMA.into(),
            file: "src/canonical.rs".into(),
            term: "value".into(),
            compilation: required.compilation.clone(),
            fact_key: required.fact.fact_key.clone(),
            direct_posting_complete: true,
        };
        let payload = json!({
            "kind":"declaration",
            "name":"value",
            "file":"src/canonical.rs",
            "startLine":7,
            "endLine":9
        });
        let mut retained = json!({
            "matches":[{
                "compilation":"cargo:demo",
                "factKey":"fact:value",
                "domainUri":"analysis:test",
                "payloadRef":payload_ref,
                "payload":payload
            }],
            "sources":[{
                "fileId":"src/canonical.rs",
                "windows":[{"startLine":1,"endLine":20,"text":"source"}]
            }]
        });
        validate_exact_selection(&selection, &retained, &required).unwrap();

        retained["matches"][0]["domainUri"] = json!("analysis:other");
        assert!(validate_exact_selection(&selection, &retained, &required).is_err());
        retained["matches"][0]["domainUri"] = json!("analysis:test");
        retained["sources"][0]["windows"][0]["startLine"] = json!(11);
        assert!(validate_exact_selection(&selection, &retained, &required).is_err());
        retained["sources"][0]["windows"][0]["startLine"] = json!(1);
        retained["matches"][0]["payloadRef"]["digest"] =
            json!(format!("sha256:{}", "f".repeat(64)));
        assert!(validate_exact_selection(&selection, &retained, &required).is_err());
    }

    #[test]
    fn exact_selection_batch_authority_is_bounded_and_unique() {
        let selection = |term: &str| {
            json!({
                "schema":EXACT_SELECTION_SCHEMA,
                "file":"src/cas.rs",
                "term":term,
                "compilation":"cargo:demo",
                "factKey":format!("fact:{term}"),
                "directPostingComplete":true
            })
        };
        let evidence = json!({
            "exactSelections":[selection("CONST1"), selection("CONST2"), selection("CONST3")]
        });
        let selections = exact_selection_authorities(&evidence).unwrap();
        assert_eq!(
            selections
                .iter()
                .map(|selection| selection.term.as_str())
                .collect::<Vec<_>>(),
            vec!["CONST1", "CONST2", "CONST3"]
        );

        let duplicate = json!({"exactSelections":[selection("CONST1"), selection("CONST1")]});
        assert!(exact_selection_authorities(&duplicate).is_err());
        let oversized = json!({
            "exactSelections":[
                selection("CONST1"),
                selection("CONST2"),
                selection("CONST3"),
                selection("CONST4")
            ]
        });
        assert!(exact_selection_authorities(&oversized).is_err());
    }

    #[test]
    fn retained_project_version_does_not_use_analyzer_version() {
        let model = json!({
            "schema":"codeclew-project-native-model/2.0", "compilation":":/main",
            "projectSemantics":{"projectCompilerVersion":"1.9.25"},
            "semanticEngine":{"analyzerCompilerVersion":"2.4.10"}
        });
        assert_eq!(
            super::retained_kotlin_compiler_version(&model, ":/main").unwrap(),
            "1.9.25"
        );
        assert!(super::retained_kotlin_compiler_version(&model, ":other/main").is_err());
        let mut missing = model.clone();
        missing["projectSemantics"] = json!({});
        assert!(super::retained_kotlin_compiler_version(&missing, ":/main").is_err());
        let context = json!({"projectCompilerVersions":{":/main":"1.9.25"}});
        assert_eq!(
            bounded_projection(&context).unwrap()["projectCompilerVersions"],
            context["projectCompilerVersions"]
        );
    }

    #[test]
    fn bounded_projection_preserves_multi_compilation_authority() {
        let context = json!({
            "language":"language:rust",
            "snapshot":{"compilations":[]},
            "task":{"intent":"inspect"},
            "compilations":[":a/main",":b/main"],
            "compilerVersions":{":a/main":"2.4.10",":b/main":"2.4.10"},
            "generationAuthority":{},
            "matches":[],
            "sources":[],
            "completeness":{},
            "verificationObligations":[],
        });
        let projection = bounded_projection(&context).unwrap();
        assert_eq!(projection["language"], "language:rust");
        assert_eq!(projection["compilations"], context["compilations"]);
        assert_eq!(projection["compilerVersions"], context["compilerVersions"]);
        assert!(projection.get("compilation").is_none());
        assert!(projection.get("compilerVersion").is_none());
    }

    #[test]
    fn bounded_projection_preserves_python_authority() {
        let context = json!({
            "language":"language:python",
            "snapshot":{"compilations":[]},
            "task":{"intent":"inspect"},
            "compilations":["python:.#backend"],
            "compilerVersions":{"python:.#backend":"tree-sitter-python-0.25.0"},
            "generationAuthority":{},
            "matches":[],
            "sources":[],
            "completeness":{},
            "verificationObligations":[],
        });
        let projection = bounded_projection(&context).unwrap();
        assert_eq!(projection["language"], "language:python");
        assert_eq!(projection["compilations"], context["compilations"]);
    }

    #[test]
    fn context_language_allowlist_is_explicit() {
        assert!(super::supported_context_language("language:java"));
        assert!(super::supported_context_language("language:javascript"));
        assert!(super::supported_context_language("language:kotlin"));
        assert!(super::supported_context_language("language:python"));
        assert!(super::supported_context_language("language:rust"));
        assert!(super::supported_context_language("language:typescript"));
        assert!(!super::supported_context_language("language:unknown"));
    }

    #[test]
    fn projection_is_bounded_and_source_windows_fail_closed() {
        let reference = json!({
            "schema":crate::cas::CAS_OBJECT_SCHEMA,
            "objectSchema":"codeclew-repository-input-blob/2.0",
            "digest":format!("sha256:{}", "a".repeat(64)),
            "size":100_000,
        });
        let combined = "first\nCODECLEW_OMITTED_LINES_BEFORE_100\nsecond";
        let source = json!({
            "fileId":"src/Target.kt",
            "contentRef":reference,
            "startLine":10,
            "endLine":110,
            "text":combined,
            "windows":[
                {"startLine":10,"endLine":20,"text":"first"},
                {"startLine":100,"endLine":110,"text":"second"},
            ],
            "completeFile":false,
        });
        validate_source_rows(&json!([source.clone()])).unwrap();
        let mut overlapping = source.clone();
        overlapping["windows"][1]["startLine"] = json!(20);
        assert!(validate_source_rows(&json!([overlapping])).is_err());

        let context = json!({
            "snapshot":{},
            "task":{},
            "compilations":[":/main"],
            "compilerVersions":{},
            "generationAuthority":{},
            "matches":(0..128).map(|index| json!({"row":index,"payload":"x".repeat(1024)})).collect::<Vec<_>>(),
            "sources":[source],
            "completeness":{},
            "verificationObligations":[],
        });
        let projection = bounded_projection(&context).unwrap();
        assert!(crate::canonical::bytes(&projection).unwrap().len() <= PROJECTION_TARGET_BYTES);
        assert_eq!(projection["truncated"], true);
    }

    #[test]
    fn oversized_match_does_not_starve_later_bounded_evidence() {
        let mut matches = (0..64)
            .map(|index| {
                json!({
                    "factKey":format!("large-{index:02}"),
                    "payload":"x".repeat(PROJECTION_TARGET_BYTES),
                })
            })
            .collect::<Vec<_>>();
        matches.push(json!({"factKey":"small","payload":{"name":"Target"}}));
        let context = json!({
            "snapshot":{},
            "task":{},
            "compilations":[":/main"],
            "compilerVersions":{},
            "generationAuthority":{},
            "sources":[],
            "matches":matches,
            "completeness":{},
            "verificationObligations":[],
        });

        let projection = bounded_projection(&context).unwrap();
        assert_eq!(projection["truncated"], true);
        assert_eq!(projection["matches"].as_array().unwrap().len(), 1);
        assert_eq!(projection["matches"][0]["factKey"], "small");
        assert!(crate::canonical::bytes(&projection).unwrap().len() <= PROJECTION_TARGET_BYTES);
    }

    #[test]
    fn large_source_does_not_starve_structured_match_authority() {
        let mut context = json!({
            "snapshot":{},
            "task":{},
            "compilations":[":/main"],
            "compilerVersions":{},
            "generationAuthority":{},
            "sources":[{"fileId":"src/Large.kt","text":"x".repeat(42 * 1024)}],
            "matches":[{
                "factKey":"kotlin:descriptor:authority",
                "payload":{"name":"Target","shape":"y".repeat(16 * 1024)},
            }],
            "completeness":{},
            "verificationObligations":[],
        });

        let matches = context["matches"].take();
        let source_only = bounded_projection(&context).unwrap();
        assert_eq!(source_only["truncated"], false);
        assert_eq!(source_only["sources"].as_array().unwrap().len(), 1);

        context["matches"] = matches;
        let projection = bounded_projection(&context).unwrap();
        assert_eq!(projection["truncated"], true);
        assert_eq!(projection["matches"].as_array().unwrap().len(), 1);
        assert!(projection["sources"].as_array().unwrap().is_empty());
        assert!(crate::canonical::bytes(&projection).unwrap().len() <= PROJECTION_TARGET_BYTES);
    }

    #[test]
    fn large_aggregate_fact_does_not_starve_exact_identity_navigation() {
        let mut aggregate = json!({
            "compilation":":/main",
            "factKey":"generic:file:aggregate",
            "payload":{
                "name":vec!["Target"; 256],
                "padding":"",
            },
        });
        let descriptor = json!({
            "compilation":":/main",
            "factKey":"generic:descriptor:exact",
            "payload":{
                "symbolIdentity":"python-syntax:src/sample.py#function:Target@10-20",
                "shape":"x".repeat(512),
            },
        });
        let mut context = json!({
            "snapshot":{},
            "task":{},
            "compilations":[":/main"],
            "compilerVersions":{},
            "generationAuthority":{},
            "sources":[],
            "matches":[aggregate.clone()],
            "completeness":{},
            "verificationObligations":[],
        });
        let initial_size = crate::canonical::bytes(&bounded_projection(&context).unwrap())
            .unwrap()
            .len();
        let padding = PROJECTION_TARGET_BYTES
            .checked_sub(initial_size + 64)
            .expect("aggregate fixture must leave a narrow stdout remainder");
        aggregate["payload"]["padding"] = json!("x".repeat(padding));
        context["matches"] = json!([aggregate.clone()]);
        let aggregate_only = bounded_projection(&context).unwrap();
        assert_eq!(aggregate_only["truncated"], false);
        assert_eq!(aggregate_only["matches"].as_array().unwrap().len(), 1);

        let ranked =
            rank_fact_evidence(vec![aggregate, descriptor.clone()], &["Target".into()], 2).unwrap();
        assert_eq!(ranked[0]["factKey"], descriptor["factKey"]);
        context["matches"] = json!(ranked);
        let projection = bounded_projection(&context).unwrap();
        assert_eq!(projection["truncated"], true);
        assert_eq!(projection["matches"].as_array().unwrap().len(), 1);
        assert_eq!(projection["matches"][0]["factKey"], descriptor["factKey"]);
        assert!(crate::canonical::bytes(&projection).unwrap().len() <= PROJECTION_TARGET_BYTES);
    }

    #[test]
    fn semantic_identity_grammars_and_unicode_share_query_normalization() {
        for identity in [
            "python-syntax:src/sample.py#function:work@10-20",
            "rust-syntax:src/lib.rs#function:work@30-40",
            "kotlin:sample/Café@50-60",
        ] {
            let term = if identity.contains("Café") {
                "café"
            } else {
                "work"
            };
            assert!(super::exact_identity_match(
                &json!({"symbolIdentity":identity}),
                term,
                None,
                0,
            ));
        }
        assert!(!super::exact_identity_match(
            &json!({"symbolIdentity":"rust-syntax:src/lib.rs#function:worker@30-40"}),
            "work",
            None,
            0,
        ));
    }

    #[test]
    fn camel_case_identity_aliases_keep_exact_declarations_ahead_of_references() {
        let declaration = json!({
            "compilation":"tsconfig:tsconfig.json",
            "factKey":"declaration",
            "payload":{
                "kind":"DECLARATION",
                "symbolIdentity":"ts:src/hooks.ts#function:usePersistentState@10-80",
            },
        });
        let reference = json!({
            "compilation":"tsconfig:tsconfig.json",
            "factKey":"reference",
            "payload":{
                "kind":"RELATION",
                "sourceIdentity":"ts:src/page.ts#function:render@90-120",
                "targetIdentity":"ts:src/hooks.ts#function:usePersistentState@10-80",
                "description":"use persistent state persistent state",
            },
        });
        let terms = ["persistent", "state", "use", "usepersistentstate"]
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let ranked = rank_fact_evidence(vec![reference, declaration], &terms, 2).unwrap();
        assert_eq!(ranked[0]["factKey"], "declaration");
    }

    #[test]
    fn lowercase_declaration_kind_and_snake_case_query_prioritize_the_named_symbol() {
        let declaration = json!({
            "compilation":"cargo:clew",
            "factKey":"declaration",
            "payload":{
                "kind":"declaration",
                "name":"aggregateCompleteness",
                "symbolIdentity":"rust-syntax:src/context.rs#function:aggregateCompleteness@10-80",
            },
        });
        let reference = json!({
            "compilation":"cargo:clew",
            "factKey":"reference",
            "payload":{
                "kind":"relation",
                "operation":"aggregate_completeness aggregate_completeness",
            },
        });
        let ranked = rank_fact_evidence(
            vec![reference, declaration],
            &["aggregate_completeness".into()],
            2,
        )
        .unwrap();
        assert_eq!(ranked[0]["factKey"], "declaration");
    }

    #[test]
    fn multi_term_snake_identity_coverage_beats_generic_direct_name() {
        let target = json!({
            "compilation":"cargo:clew",
            "factKey":"target",
            "payload":{
                "kind":"declaration",
                "name":"aggregate_unmatched_terms_are_global_not_member_local",
                "symbolIdentity":"rust-syntax:src/context.rs#function:aggregate_unmatched_terms_are_global_not_member_local@100-200",
            },
        });
        let generic = json!({
            "compilation":"cargo:clew",
            "factKey":"generic",
            "payload":{
                "kind":"declaration",
                "name":"member",
                "symbolIdentity":"rust-syntax:src/context.rs#function:member@10-20",
            },
        });
        let reference = json!({
            "compilation":"cargo:clew",
            "factKey":"reference",
            "payload":{
                "kind":"relation",
                "targetIdentity":"rust-syntax:src/context.rs#function:aggregate_unmatched_terms_are_global_not_member_local@100-200",
            },
        });
        let terms = ["aggregate", "unmatched", "global", "member"]
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let ranked = rank_fact_evidence(vec![generic, reference, target], &terms, 3).unwrap();
        assert_eq!(ranked[0]["factKey"], "target");
    }

    #[test]
    fn snake_identity_component_selects_its_declaration_range() {
        let mut source = "const global = \"early lexical occurrence\";\n".to_owned();
        source.push_str(&"padding\n".repeat(100));
        let declaration_start = source.len();
        source.push_str("fn aggregate_unmatched_terms_are_global_not_member_local() {\n");
        source.push_str("    let marker = \"DECLARATION_BODY\";\n}\n");
        let facts = vec![json!({
            "payload":{
                "kind":"declaration",
                "symbolIdentity":"rust-syntax:src/context.rs#function:aggregate_unmatched_terms_are_global_not_member_local@100-200",
                "file":"src/context.rs",
                "rangeStart":declaration_start,
                "rangeEnd":source.len(),
            },
        })];
        let hints = source_offset_hints(&facts, &["global".into()]);
        let windows = source_windows(&source, &["global".into()], hints.get("src/context.rs"));
        assert_eq!(windows.len(), 1);
        assert!(windows[0].2.contains("DECLARATION_BODY"));
        assert!(!windows[0].2.contains("early lexical occurrence"));
    }

    #[test]
    fn exact_identity_navigation_never_upgrades_unsure_generation_authority() {
        let query = super::AggregateQueryContext {
            schema: AGGREGATE_QUERY_CONTEXT_SCHEMA.into(),
            index_id: "sha256:index".into(),
            requested_terms: vec!["work".into()],
            unmatched_terms: Vec::new(),
            facts: Vec::new(),
            query_shards_read: 1,
            truncated: false,
        };
        let evidence = vec![json!({
            "payload":{
                "symbolIdentity":"rust-syntax:src/lib.rs#function:work@30-40",
            },
        })];
        assert!(!super::query_selection_verified(
            "UNSURE", &query, &evidence
        ));
        assert!(super::query_selection_verified(
            "VERIFIED", &query, &evidence
        ));
        let mut truncated = query;
        truncated.truncated = true;
        assert!(!super::query_selection_verified(
            "VERIFIED", &truncated, &evidence,
        ));
    }

    #[test]
    fn source_projection_preserves_ranked_evidence_path_order() {
        let facts = vec![
            json!({"payload":{"file":"src/main/MainA.kt"}}),
            json!({"payload":{"file":"src/test/TestZ.kt","path":"../unsafe"}}),
            json!({"payload":{"file":"src/main/MainB.kt","duplicate":"src/main/MainA.kt"}}),
            json!({"payload":{"file":"src/main/MainA.kt"}}),
        ];
        let paths = ordered_paths_in_evidence(&facts);
        assert_eq!(
            paths,
            vec![
                "src/main/MainA.kt",
                "src/test/TestZ.kt",
                "src/main/MainB.kt",
            ]
        );
        assert_eq!(ordered_paths_in_evidence(&facts), paths);

        let source = |file: &str, bytes: usize| {
            json!({
                "fileId":file,
                "contentRef":{},
                "text":"x".repeat(bytes),
                "windows":[],
            })
        };
        let context = json!({
            "snapshot":{},
            "task":{},
            "compilations":[":main",":test"],
            "compilerVersions":{},
            "generationAuthority":{},
            "sources":[
                source("src/main/MainA.kt", 30 * 1024),
                source("src/test/TestZ.kt", 1024),
                source("src/main/MainB.kt", 30 * 1024),
            ],
            "matches":[],
            "completeness":{},
            "verificationObligations":[],
        });
        let projection = bounded_projection(&context).unwrap();
        assert_eq!(projection["truncated"], true);
        assert_eq!(
            projection["sources"]
                .as_array()
                .unwrap()
                .iter()
                .map(|source| source["fileId"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["src/main/MainA.kt", "src/test/TestZ.kt"]
        );
        assert!(crate::canonical::bytes(&projection).unwrap().len() <= PROJECTION_TARGET_BYTES);
    }

    #[test]
    fn aggregate_query_retains_deterministic_compilation_provenance() {
        let root = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(root.path().join("v2")).unwrap();
        let store = CasStore::open(&state).unwrap();
        let payload = store.put("test/fact/1", br#"{"name":"Shared"}"#).unwrap();
        let fact = FactHit {
            fact_key: "fact:shared".into(),
            domain_uri: CapabilityUri::parse("analysis:test").unwrap(),
            payload,
        };
        let query = |index_id: &str| QueryContext {
            schema: QUERY_CONTEXT_SCHEMA.into(),
            index_id: index_id.into(),
            requested_terms: vec!["Shared".into()],
            unmatched_terms: Vec::new(),
            facts: vec![fact.clone()],
            query_shards_read: 1,
            truncated: false,
        };
        let contexts = std::collections::BTreeMap::from([
            (":b/main".into(), query("index:b")),
            (":a/main".into(), query("index:a")),
        ]);
        let aggregate = merge_query_contexts(&contexts, 16).unwrap();
        assert_eq!(aggregate.schema, AGGREGATE_QUERY_CONTEXT_SCHEMA);
        assert_eq!(aggregate.facts.len(), 2);
        assert_eq!(aggregate.facts[0].compilation, ":a/main");
        assert_eq!(aggregate.facts[1].compilation, ":b/main");
        let evidence = load_fact_evidence(&store, &aggregate.facts).unwrap();
        assert_eq!(evidence[0]["compilation"], ":a/main");
        assert_eq!(evidence[1]["compilation"], ":b/main");
        assert_eq!(evidence[0]["factKey"], evidence[1]["factKey"]);
        let source_ref = store.put("test/source/1", b"Shared").unwrap();
        let context = json!({
            "schema":super::BOUNDED_CONTEXT_SCHEMA,
            "language":"language:kotlin",
            "snapshot":{},
            "task":{},
            "compilations":[":a/main",":b/main"],
            "compilerVersions":{},
            "generationAuthority":{},
            "matches":evidence,
            "sources":[{
                "fileId":"src/Shared.kt",
                "contentRef":source_ref,
                "startLine":1,
                "endLine":1,
                "text":"Shared",
                "windows":[{"startLine":1,"endLine":1,"text":"Shared"}],
                "completeFile":false,
            }],
            "completeness":{},
            "verificationObligations":[],
        });
        let mut projection = bounded_projection(&context).unwrap();
        let envelope = json!({
            "schema":super::BOUNDED_CONTEXT_EVIDENCE_SCHEMA,
            "context":context,
            "queryContext":aggregate,
            "queryContexts":contexts,
            "stdoutCompleteness":{},
        });
        super::validate_context_payload(&projection, &envelope).unwrap();
        let original = projection.clone();

        let mut proper_subset = original.clone();
        proper_subset["matches"].as_array_mut().unwrap().pop();
        proper_subset["truncated"] = json!(true);
        super::validate_context_payload(&proper_subset, &envelope).unwrap();
        proper_subset["truncated"] = json!(false);
        assert!(super::validate_context_payload(&proper_subset, &envelope).is_err());

        let mut reordered = original.clone();
        reordered["matches"].as_array_mut().unwrap().reverse();
        assert!(super::validate_context_payload(&reordered, &envelope).is_err());

        projection["matches"][0]
            .as_object_mut()
            .unwrap()
            .remove("compilation");
        assert!(super::validate_context_payload(&projection, &envelope).is_err());

        let mut tampered_payload = original.clone();
        tampered_payload["matches"][0]["payload"]["name"] = json!("Changed");
        assert!(super::validate_context_payload(&tampered_payload, &envelope).is_err());

        let mut tampered_source = original.clone();
        tampered_source["sources"][0]["contentRef"]["size"] = json!(999);
        assert!(super::validate_context_payload(&tampered_source, &envelope).is_err());

        let mut duplicate = original;
        let repeated = duplicate["matches"][0].clone();
        duplicate["matches"].as_array_mut().unwrap().push(repeated);
        assert!(super::validate_context_payload(&duplicate, &envelope).is_err());
    }

    #[test]
    fn aggregate_query_budget_is_shared_across_compilations() {
        let root = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(root.path().join("v2")).unwrap();
        let store = CasStore::open(&state).unwrap();
        let payload = store.put("test/fact/1", br#"{"name":"Shared"}"#).unwrap();
        let main_facts = (0..32)
            .map(|index| FactHit {
                fact_key: format!("fact:main:{index:02}"),
                domain_uri: CapabilityUri::parse("analysis:test").unwrap(),
                payload: payload.clone(),
            })
            .collect::<Vec<_>>();
        let shared = main_facts[0].clone();
        let query = |index_id: &str, facts: Vec<FactHit>, truncated: bool| QueryContext {
            schema: QUERY_CONTEXT_SCHEMA.into(),
            index_id: index_id.into(),
            requested_terms: vec!["shared".into()],
            unmatched_terms: if facts.is_empty() {
                vec!["shared".into()]
            } else {
                Vec::new()
            },
            facts,
            query_shards_read: 1,
            truncated,
        };
        let contexts = BTreeMap::from([
            (
                ":z/test".into(),
                query("index:test", vec![shared.clone()], false),
            ),
            (
                ":a/main".into(),
                query("index:main", main_facts.clone(), false),
            ),
            (":m/empty".into(), query("index:empty", Vec::new(), false)),
        ]);
        let aggregate = merge_query_contexts(&contexts, 2).unwrap();
        assert_eq!(aggregate.facts.len(), 2);
        assert_eq!(aggregate.facts[0].compilation, ":a/main");
        assert_eq!(aggregate.facts[1].compilation, ":z/test");
        assert_eq!(aggregate.facts[0].fact, aggregate.facts[1].fact);
        assert!(aggregate.truncated);
        assert!(aggregate.unmatched_terms.is_empty());
        assert_eq!(
            aggregate,
            merge_query_contexts(&contexts, aggregate.facts.len()).unwrap()
        );

        let three_lanes = BTreeMap::from([
            (
                ":z/test".into(),
                query("index:test", vec![shared.clone()], false),
            ),
            (":a/main".into(), query("index:main", main_facts, false)),
            (
                ":m/integration".into(),
                query("index:integration", vec![shared], false),
            ),
        ]);
        let aggregate = merge_query_contexts(&three_lanes, 3).unwrap();
        assert_eq!(
            aggregate
                .facts
                .iter()
                .map(|hit| hit.compilation.as_str())
                .collect::<Vec<_>>(),
            vec![":a/main", ":m/integration", ":z/test"]
        );
        let one = merge_query_contexts(&three_lanes, 1).unwrap();
        assert_eq!(one.facts.len(), 1);
        assert_eq!(one.facts[0].compilation, ":a/main");

        let lane_truncated = BTreeMap::from([(
            ":a/main".into(),
            query("index:main", vec![aggregate.facts[0].fact.clone()], true),
        )]);
        assert!(merge_query_contexts(&lane_truncated, 16).unwrap().truncated);
    }

    #[test]
    fn ranked_evidence_budget_is_shared_across_compilations() {
        let main = (0..32)
            .map(|index| {
                json!({
                    "compilation":":a/main",
                    "factKey":format!("main:{index:02}"),
                    "payload":{
                        "symbolIdentity":"Target",
                        "ownerIdentity":"Target",
                    },
                })
            })
            .collect::<Vec<_>>();
        let test = json!({
            "compilation":":z/test",
            "factKey":"test:00",
            "payload":{"name":"Target"},
        });
        let mut combined = main.clone();
        combined.push(test.clone());
        let ranked = rank_fact_evidence(combined, &["Target".into()], 2).unwrap();
        assert_eq!(ranked.len(), 2);
        assert_eq!(ranked[0]["compilation"], ":a/main");
        assert_eq!(ranked[1]["compilation"], ":z/test");

        let single = rank_fact_evidence(
            vec![
                json!({
                    "compilation":":a/main",
                    "factKey":"a-low",
                    "payload":{"name":"Target"},
                }),
                json!({
                    "compilation":":a/main",
                    "factKey":"z-high",
                    "payload":{
                        "symbolIdentity":"Target",
                        "ownerIdentity":"Target",
                    },
                }),
            ],
            &["Target".into()],
            2,
        )
        .unwrap();
        assert_eq!(single[0]["factKey"], "z-high");
        assert_eq!(single[1]["factKey"], "a-low");

        let no_identity = rank_fact_evidence(
            vec![
                json!({
                    "compilation":":a/main",
                    "factKey":"a-low",
                    "payload":{"name":"Target"},
                }),
                json!({
                    "compilation":":a/main",
                    "factKey":"z-high",
                    "payload":{"name":["Target", "Target"]},
                }),
            ],
            &["Target".into()],
            2,
        )
        .unwrap();
        assert_eq!(no_identity[0]["factKey"], "z-high");
        assert_eq!(no_identity[1]["factKey"], "a-low");

        let shared_payload = json!({"symbolIdentity":"Target"});
        let three = rank_fact_evidence(
            vec![
                json!({"compilation":":z/test","factKey":"same","payload":shared_payload.clone()}),
                json!({"compilation":":a/main","factKey":"same","payload":shared_payload.clone()}),
                json!({"compilation":":m/integration","factKey":"same","payload":shared_payload}),
            ],
            &["Target".into()],
            3,
        )
        .unwrap();
        assert_eq!(
            three
                .iter()
                .map(|fact| fact["compilation"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec![":a/main", ":m/integration", ":z/test"]
        );
        assert!(
            rank_fact_evidence(
                vec![json!({"factKey":"missing","payload":{}})],
                &["Target".into()],
                1,
            )
            .is_err()
        );
        assert!(
            rank_fact_evidence(
                vec![json!({"compilation":1,"factKey":"invalid","payload":{}})],
                &["Target".into()],
                1,
            )
            .is_err()
        );
    }

    #[test]
    fn declaration_range_beats_an_earlier_import_match() {
        let mut source = "import sample.Target\n".to_owned();
        source.push_str(&"padding\n".repeat(2_100));
        let declaration_offset = source.len();
        source.push_str("class Target {\n    fun changed() = true\n}\n");
        let facts = vec![json!({
            "payload": {
                "declarations": [{
                    "name": "Target",
                    "file": "src/Target.kt",
                    "rangeStart": declaration_offset,
                    "rangeEnd": source.len(),
                }]
            }
        })];
        let hints = source_offset_hints(&facts, &["Target".into()]);
        let windows = source_windows(&source, &["Target".into()], hints.get("src/Target.kt"));
        assert_eq!(windows.len(), 1);
        let text = &windows[0].2;
        assert!(text.contains("class Target"));
        assert!(!text.contains("import sample.Target"));
    }

    #[test]
    fn reference_follow_returns_a_complete_small_source_file() {
        let source = "fn bytes() { value(); }\nfn value() { sort_value(); }\nfn sort_value() {}\n";
        let ranges = BTreeMap::from([(0usize, Some(23usize))]);
        let windows = source_windows_with_policy(source, &["bytes".into()], Some(&ranges), true);
        assert_eq!(windows, vec![(1, 3, source.into())]);

        let bounded = source_windows(source, &["bytes".into()], Some(&ranges));
        assert_eq!(bounded.len(), 1);
        assert!(bounded[0].2.len() <= source.len());
    }

    #[test]
    fn declaration_range_includes_a_body_beyond_the_default_window() {
        let mut source = "import sample.Target\n".to_owned();
        source.push_str(&"padding\n".repeat(2_100));
        let declaration_offset = source.len();
        source.push_str("fn Target() {\n");
        source.push_str(&"    let value = 1;\n".repeat(80));
        source.push_str("    let marker = \"DECLARATION_TAIL\";\n}\n");
        let declaration_end = source.len();
        let facts = vec![json!({
            "payload": {
                "name": "Target",
                "file": "src/target.rs",
                "rangeStart": declaration_offset,
                "rangeEnd": declaration_end,
            }
        })];
        let hints = source_offset_hints(&facts, &["Target".into()]);
        let windows = source_windows(&source, &["Target".into()], hints.get("src/target.rs"));
        assert_eq!(windows.len(), 1);
        let text = &windows[0].2;
        assert!(text.contains("DECLARATION_TAIL"));
        assert!(!text.contains("padding"));
        assert!(text.len() <= MAX_SOURCE_BYTES);
    }

    #[test]
    fn denser_lexical_support_is_retained_beside_declaration_windows() {
        let mut source = "fn evidence_value(member_completeness: &[Value]) {\n".to_owned();
        source.push_str("    json!({\"memberCompleteness\":member_completeness});\n}\n");
        source.push_str(&"padding\n".repeat(40));
        let aggregate_start = source.len();
        source.push_str("fn aggregate_completeness(member_rows: &[Value]) {\n    body();\n}\n");
        let aggregate_end = source.len();
        source.push_str(&"padding\n".repeat(40));
        let test_start = source.len();
        source.push_str(
            "fn aggregate_unmatched_terms_are_global_not_member_local() {\n    assert!(true);\n}\n",
        );
        let test_end = source.len();
        let ranges = BTreeMap::from([
            (aggregate_start, Some(aggregate_end)),
            (test_start, Some(test_end)),
        ]);
        let windows = source_windows(
            &source,
            &[
                "unmatched".into(),
                "global".into(),
                "member".into(),
                "completeness".into(),
            ],
            Some(&ranges),
        );
        assert_eq!(windows.len(), 3);
        assert_eq!(
            windows[0].2,
            "    json!({\"memberCompleteness\":member_completeness});"
        );
        assert!(windows[1].2.contains("fn aggregate_completeness"));
        assert!(
            windows[2]
                .2
                .contains("aggregate_unmatched_terms_are_global_not_member_local")
        );
    }

    #[test]
    fn repeated_support_term_sets_reuse_one_retained_score() {
        let mut source = "fn member_completeness() {}\n".to_owned();
        source.push_str(&"memberCompleteness member_completeness\n".repeat(1_000));
        let lines = source.lines().collect::<Vec<_>>();
        let windows = vec![(1, 1, lines[0].to_owned())];
        let (support, retained_score_computations) =
            best_lexical_support(&lines, &windows, &["member".into(), "completeness".into()]);
        assert_eq!(retained_score_computations, 1);
        assert_eq!(support.map(|(_, line, _)| line), Some(2));
    }

    #[test]
    fn nearby_short_declarations_retain_their_bounded_owner_context() {
        let mut source =
            "fn collect_member_completeness(contexts: &[MemberContext]) {}\n".to_owned();
        source.push_str("let member_unmatched = completeness;\n");
        source.push_str("\"memberCompleteness\": member_completeness,\n");
        source.push_str("fn aggregate_unmatched_terms_are_global_not_member_local() {}\n");
        let lines = source.lines().collect::<Vec<_>>();
        let ranges = BTreeMap::from([
            (0, Some(lines[0].len())),
            (
                source.rfind(lines[3]).unwrap(),
                Some(source.rfind(lines[3]).unwrap() + lines[3].len()),
            ),
        ]);
        let windows = source_windows(
            &source,
            &[
                "unmatched".into(),
                "global".into(),
                "member".into(),
                "completeness".into(),
            ],
            Some(&ranges),
        );
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].0, 1);
        assert_eq!(windows[0].1, 4);
        assert!(windows[0].2.contains("let member_unmatched"));
        assert!(windows[0].2.contains("\"memberCompleteness\""));
    }

    #[test]
    fn contextual_and_merged_windows_preserve_crlf_snapshot_bytes() {
        let source = (1..=60)
            .map(|line| match line {
                10 => "fn Target() {}".to_owned(),
                30 => "fn Second() {}".to_owned(),
                _ => format!("padding_{line}"),
            })
            .collect::<Vec<_>>()
            .join("\r\n");
        let target_start = source.find("fn Target").unwrap();
        let second_start = source.find("fn Second").unwrap();
        let ranges = BTreeMap::from([
            (target_start, Some(target_start + "fn Target() {}".len())),
            (second_start, Some(second_start + "fn Second() {}".len())),
        ]);
        let windows = source_windows(&source, &["Target".into(), "Second".into()], Some(&ranges));

        assert_eq!(windows.len(), 1);
        assert!(windows[0].2.contains("\r\n"));
        assert!(!windows[0].2.replace("\r\n", "").contains('\n'));
        assert!(source.contains(&windows[0].2));
    }

    #[test]
    fn compiler_identity_and_start_offset_beat_an_earlier_import_match() {
        let mut source = "import sample.Target\n".to_owned();
        source.push_str(&"padding\n".repeat(2_100));
        let declaration_offset = source.len();
        source.push_str("class Target {\n    fun changed() = true\n}\n");
        let facts = vec![json!({
            "payload": {
                "compilerClassId": "sample/Target",
                "file": "src/Target.kt",
                "start": declaration_offset,
                "end": source.len(),
            }
        })];
        let hints = source_offset_hints(&facts, &["Target".into()]);
        let windows = source_windows(&source, &["Target".into()], hints.get("src/Target.kt"));
        assert_eq!(windows.len(), 1);
        let text = &windows[0].2;
        assert!(text.contains("class Target"));
        assert!(!text.contains("import sample.Target"));
    }

    #[test]
    fn multiple_identity_ranges_become_bounded_disjoint_windows() {
        let mut source = "import sample.Target\n".to_owned();
        source.push_str(&"before\n".repeat(3_000));
        let first_offset = source.len();
        source.push_str("class Target { val first = \"FIRST_WINDOW\" }\n");
        source.push_str(&"between\n".repeat(3_000));
        let second_offset = source.len();
        source.push_str("fun Target.second() = \"SECOND_WINDOW\"\n");
        let facts = vec![
            json!({"payload": {
                "compilerClassId": "sample/Target",
                "file": "src/Target.kt",
                "start": first_offset,
            }}),
            json!({"payload": {
                "ownerIdentity": "class:sample/Target",
                "file": "src/Target.kt",
                "start": second_offset,
            }}),
        ];
        let hints = source_offset_hints(&facts, &["Target".into()]);
        let windows = source_windows(&source, &["Target".into()], hints.get("src/Target.kt"));
        assert_eq!(windows.len(), 2);
        assert!(windows[0].2.contains("FIRST_WINDOW"));
        assert!(windows[1].2.contains("SECOND_WINDOW"));
        assert!(!windows[0].2.contains("SECOND_WINDOW"));
        assert!(!windows[1].2.contains("FIRST_WINDOW"));
        assert!(
            windows.iter().map(|window| window.2.len()).sum::<usize>() <= super::MAX_SOURCE_BYTES
        );
    }

    #[test]
    fn source_range_hints_keep_the_highest_ranked_late_declaration() {
        let mut source = String::new();
        let mut ranges = Vec::new();
        for marker in [
            "EARLY_ONE",
            "EARLY_TWO",
            "EARLY_THREE",
            "EARLY_FOUR",
            "TARGET",
        ] {
            let start = source.len();
            source.push_str(&format!("fn Target() {{ let marker = \"{marker}\"; }}\n"));
            ranges.push((start, source.len()));
            source.push_str(&"padding\n".repeat(40));
        }
        let facts = std::iter::once(ranges[4])
            .chain(ranges[..4].iter().copied())
            .map(|(start, end)| {
                json!({
                    "payload":{
                        "kind":"declaration",
                        "name":"Target",
                        "file":"src/target.rs",
                        "rangeStart":start,
                        "rangeEnd":end,
                    },
                })
            })
            .collect::<Vec<_>>();

        let hints = source_offset_hints(&facts, &["Target".into()]);
        let windows = source_windows(&source, &["Target".into()], hints.get("src/target.rs"));
        let retained = windows
            .iter()
            .map(|window| window.2.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(retained.contains("TARGET"));
        assert!(!retained.contains("EARLY_FOUR"));
        assert!(windows.len() <= super::MAX_SOURCE_WINDOWS);
    }

    #[test]
    fn oversized_fact_is_retained_as_a_bounded_opaque_reference() {
        let root = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(root.path().join("v2")).unwrap();
        let store = CasStore::open(&state).unwrap();
        let payload = store
            .put("test/large-fact/1", &vec![b'x'; MAX_PAYLOAD_BYTES + 1])
            .unwrap();
        let fact = FactHit {
            fact_key: "fact:large".into(),
            domain_uri: CapabilityUri::parse("analysis:test").unwrap(),
            payload,
        };
        let evidence = load_fact_evidence(
            &store,
            &[CompilationFactHit {
                compilation: ":/main".into(),
                fact,
            }],
        )
        .unwrap();
        assert_eq!(evidence[0]["compilation"], ":/main");
        assert_eq!(
            evidence[0]["payload"]["reason"],
            "PAYLOAD_EXCEEDS_CONTEXT_LIMIT"
        );
        assert_eq!(
            evidence[0]["payload"]["size"],
            (MAX_PAYLOAD_BYTES + 1) as u64
        );
    }
}
