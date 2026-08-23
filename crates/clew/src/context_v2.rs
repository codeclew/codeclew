use crate::canonical;
use crate::cas::{CasObject, CasStore};
use crate::error::{ClewError, ErrorCode};
use crate::generation_service::{
    ensure_session_generation, load_query_index, load_session_generation, load_snapshot,
};
use crate::query_v2::{FactHit, QueryContext, expand, query};
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
const MAX_SOURCE_WINDOWS: usize = 4;
const PROJECTION_TARGET_BYTES: usize = 54 * 1024;
pub const AGGREGATE_QUERY_CONTEXT_SCHEMA: &str = "codeclew-aggregate-query-context/1.0";
pub const BOUNDED_CONTEXT_SCHEMA: &str = "codeclew-bounded-context/4.0";
pub const BOUNDED_CONTEXT_EVIDENCE_SCHEMA: &str = "codeclew-bounded-context-evidence/4.0";
pub const BOUNDED_CONTEXT_PROJECTION_SCHEMA: &str = "codeclew-bounded-context-projection/4.0";

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
    if compilation_set.is_empty()
        || compilation_set.len() != compilations.len()
        || queries.keys().ne(compilation_set.iter())
        || projection.get("compilations") != evidence.pointer("/context/compilations")
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
    let recomputed = merge_query_contexts(&queries, aggregate.facts.len().max(1))?;
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
    for compilation in &ready.compilations {
        let index = load_query_index(&store, compilation)?;
        let context = if let Some(parent_queries) = &parent_queries {
            let parent_query = parent_queries
                .get(&compilation.compilation)
                .ok_or_else(|| invalid("parent context misses a selected compilation"))?;
            expand(&store, &index, parent_query, terms, fact_limit)?
        } else {
            query(&store, &index, terms, fact_limit)?
        };
        query_contexts.insert(compilation.compilation.clone(), context);
    }
    let query_context = merge_query_contexts(&query_contexts, fact_limit)?;
    let snapshot = load_snapshot(&store, &ready)?;
    let selection_terms = &query_context.requested_terms;
    let evidence_facts = rank_fact_evidence(
        load_fact_evidence(&store, &query_context.facts)?,
        selection_terms,
        max_roots.saturating_mul(4),
    )?;
    let paths = ordered_paths_in_evidence(&evidence_facts);
    let source_hints = source_offset_hints(&evidence_facts, selection_terms);
    let sources = load_source_snippets(&store, &snapshot, &paths, selection_terms, &source_hints)?;
    let verified = ready.certainty == "VERIFIED"
        && !query_context.truncated
        && query_context.unmatched_terms.is_empty()
        && !evidence_facts.is_empty()
        && query_context.requested_terms.iter().all(|term| {
            evidence_facts
                .iter()
                .any(|fact| exact_identity_match(&fact["payload"], term, None, 0))
        });
    let conditional = !verified && !query_context.facts.is_empty();
    let mut obligations = ready
        .obligations
        .iter()
        .map(|obligation| json!({
            "id":obligation,
            "code":"UNSURE_GENERATION_AUTHORITY",
            "subject":query_context.requested_terms,
            "requiredCheckSet":["restore successful compiler-semantic analysis before publication"],
            "publicationBlocking":true,
        }))
        .collect::<Vec<_>>();
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
    let context = json!({
        "schema":BOUNDED_CONTEXT_SCHEMA,
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
    let evidence = json!({
        "schema":BOUNDED_CONTEXT_EVIDENCE_SCHEMA,
        "context":context,
        "queryContext":query_context,
        "queryContexts":query_contexts,
        "stdoutCompleteness":{
            "status":status,
            "certainty":certainty,
        },
    });
    Ok((projection, evidence))
}

fn merge_query_contexts(
    contexts: &BTreeMap<String, QueryContext>,
    fact_limit: usize,
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
    let facts = fair_compilation_selection(&lanes, fact_limit);
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

fn paths_in_payload(payload: &Value) -> Vec<String> {
    let mut paths = BTreeSet::new();
    collect_paths(payload, None, &mut paths, 0);
    paths.into_iter().collect()
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
    source_hints: &BTreeMap<String, BTreeSet<usize>>,
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
        let windows = source_windows(source, terms, source_hints.get(path));
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

fn source_offset_hints(facts: &[Value], terms: &[String]) -> BTreeMap<String, BTreeSet<usize>> {
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
    output: &mut BTreeMap<String, BTreeSet<usize>>,
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
            let exact_identity_match = terms
                .iter()
                .any(|term| exact_identity_match(value, term, None, 0));
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
                if let (Some(file), Some(offset)) = (file, offset)
                    && safe_path(file)
                    && let Ok(offset) = usize::try_from(offset)
                {
                    output.entry(file.to_owned()).or_default().insert(offset);
                }
            }
            for value in values.values() {
                collect_source_offset_hints(value, terms, output, depth + 1);
            }
        }
        _ => {}
    }
}

fn source_windows(
    source: &str,
    terms: &[String],
    source_offsets: Option<&BTreeSet<usize>>,
) -> Vec<(usize, usize, String)> {
    let offsets = source_offsets
        .filter(|offsets| !offsets.is_empty())
        .map(|offsets| offsets.iter().copied().map(Some).collect::<Vec<_>>())
        .unwrap_or_else(|| vec![None]);
    let mut windows = Vec::new();
    let mut projected_bytes = 0usize;
    let lines = source.lines().collect::<Vec<_>>();
    for offset in offsets {
        let window = snippet(source, terms, offset);
        if windows.iter().any(|existing: &(usize, usize, String)| {
            existing.0 == window.0 && existing.1 == window.1
        }) {
            continue;
        }
        if let Some(previous) = windows.last_mut()
            && window.0 <= previous.1.saturating_add(1)
        {
            let merged_end = previous.1.max(window.1);
            let merged_text = lines[previous.0 - 1..merged_end].join("\n");
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
    windows
}

fn snippet(source: &str, terms: &[String], source_offset: Option<usize>) -> (usize, usize, String) {
    if source.len() <= MAX_SNIPPET_BYTES {
        return (1, source.lines().count().max(1), source.to_owned());
    }
    let lowered_terms = terms
        .iter()
        .map(|term| term.to_lowercase())
        .collect::<Vec<_>>();
    let lines = source.lines().collect::<Vec<_>>();
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
            lines.iter().position(|line| {
                let line = line.to_lowercase();
                lowered_terms.iter().any(|term| line.contains(term))
            })
        })
        .unwrap_or(0);
    let start = hit.saturating_sub(20);
    let mut end = (hit + 40).min(lines.len());
    let mut text = lines[start..end].join("\n");
    while text.len() > MAX_SNIPPET_BYTES && end > start + 1 {
        end -= 1;
        text = lines[start..end].join("\n");
    }
    (start + 1, end, text)
}

fn bounded_projection(context: &Value) -> Result<Value, ClewError> {
    let mut projection = json!({
        "schema":BOUNDED_CONTEXT_PROJECTION_SCHEMA,
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
    let mut projected_bytes = canonical::bytes(&projection).map_err(internal)?.len();
    for key in ["sources", "matches"] {
        for value in context[key].as_array().into_iter().flatten() {
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
    let mut lanes = BTreeMap::<String, Vec<Value>>::new();
    for fact in facts {
        let compilation = fact
            .get("compilation")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("ranked context fact has no compilation provenance"))?;
        lanes.entry(compilation.to_owned()).or_default().push(fact);
    }
    for facts in lanes.values_mut() {
        facts.sort_by(|left, right| {
            let score = |value: &Value| fact_score(&value["payload"], &lowered, None, 0);
            score(right)
                .cmp(&score(left))
                .then_with(|| left["factKey"].as_str().cmp(&right["factKey"].as_str()))
        });
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

fn fact_score(value: &Value, terms: &[String], key: Option<&str>, depth: usize) -> usize {
    if depth > 32 {
        return 0;
    }
    match value {
        Value::String(value) => {
            let value = value.to_lowercase();
            let weight = match key {
                Some(
                    "symbolIdentity" | "compilerCallableId" | "compilerClassId" | "ownerIdentity",
                ) => 16,
                Some("path" | "file" | "fileId" | "relativePath" | "sourcePath") => 8,
                Some("name" | "declarationKind") => 6,
                Some("sourceSet" | "module" | "provider" | "schema") => 0,
                _ => 1,
            };
            weight
                * terms
                    .iter()
                    .filter(|term| value.contains(term.as_str()))
                    .count()
        }
        Value::Array(values) => values
            .iter()
            .map(|value| fact_score(value, terms, key, depth + 1))
            .sum(),
        Value::Object(values) => values
            .iter()
            .map(|(key, value)| fact_score(value, terms, Some(key), depth + 1))
            .sum(),
        _ => 0,
    }
}

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
                    matches!(character, '/' | '.' | '#' | ':' | '(' | ')' | ';')
                })
                .any(|component| component.eq_ignore_ascii_case(term))
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
        AGGREGATE_QUERY_CONTEXT_SCHEMA, CompilationFactHit, MAX_PAYLOAD_BYTES,
        PROJECTION_TARGET_BYTES, bounded_projection, load_fact_evidence, merge_query_contexts,
        ordered_paths_in_evidence, rank_fact_evidence, source_offset_hints, source_windows,
        validate_source_rows,
    };
    use crate::adapter_v2::CapabilityUri;
    use crate::cas::CasStore;
    use crate::query_v2::{FactHit, QUERY_CONTEXT_SCHEMA, QueryContext};
    use crate::state::StateAuthority;
    use serde_json::json;
    use std::collections::BTreeMap;

    #[test]
    fn bounded_projection_preserves_multi_compilation_authority() {
        let context = json!({
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
        assert_eq!(projection["compilations"], context["compilations"]);
        assert_eq!(projection["compilerVersions"], context["compilerVersions"]);
        assert!(projection.get("compilation").is_none());
        assert!(projection.get("compilerVersion").is_none());
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
        let context = json!({
            "schema":super::BOUNDED_CONTEXT_SCHEMA,
            "snapshot":{},
            "task":{},
            "compilations":[":a/main",":b/main"],
            "compilerVersions":{},
            "generationAuthority":{},
            "matches":evidence,
            "sources":[],
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
        projection["matches"][0]
            .as_object_mut()
            .unwrap()
            .remove("compilation");
        assert!(super::validate_context_payload(&projection, &envelope).is_err());
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
