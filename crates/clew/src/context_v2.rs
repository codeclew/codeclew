use crate::canonical;
use crate::cas::CasStore;
use crate::error::{ClewError, ErrorCode};
use crate::generation_service::{
    ensure_session_generation, load_query_index, load_session_generation, load_snapshot,
};
use crate::query_v2::{FactHit, QueryContext, expand, query};
use crate::repository_snapshot::{RepositoryInputSnapshot, WorktreeKind};
use crate::session::{ContextObject, SessionAuthority};
use crate::state::StateAuthority;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

const MAX_EVIDENCE_FACTS: usize = 4096;
const MAX_PAYLOAD_BYTES: usize = 1024 * 1024;
const MAX_SNIPPET_BYTES: usize = 16 * 1024;
const PROJECTION_TARGET_BYTES: usize = 54 * 1024;

pub fn create(
    session: &SessionAuthority,
    intent: &str,
    terms: &[String],
    max_roots: usize,
    parent: Option<&ContextObject>,
) -> Result<(Value, Value), ClewError> {
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
    let index = load_query_index(&store, &ready)?;
    let fact_limit = max_roots.saturating_mul(16).min(MAX_EVIDENCE_FACTS);
    let query_context = if let Some(parent) = parent {
        let parent_query: QueryContext = serde_json::from_value(
            parent
                .evidence
                .get("queryContext")
                .cloned()
                .ok_or_else(|| invalid("parent context has no query authority"))?,
        )
        .map_err(parse_error)?;
        expand(&store, &index, &parent_query, terms, fact_limit)?
    } else {
        query(&store, &index, terms, fact_limit)?
    };
    let snapshot = load_snapshot(&store, &ready)?;
    let evidence_facts = rank_fact_evidence(
        load_fact_evidence(&store, &query_context.facts)?,
        terms,
        max_roots.saturating_mul(4),
    );
    let paths = evidence_facts
        .iter()
        .flat_map(|fact| paths_in_payload(&fact["payload"]))
        .collect::<BTreeSet<_>>();
    let sources = load_source_snippets(&store, &snapshot, &paths, terms)?;
    let verified = !query_context.truncated
        && query_context.unmatched_terms.is_empty()
        && !evidence_facts.is_empty()
        && query_context.requested_terms.iter().all(|term| {
            evidence_facts
                .iter()
                .any(|fact| exact_identity_match(&fact["payload"], term, None, 0))
        });
    let conditional = !verified && !query_context.facts.is_empty();
    let obligations = if conditional {
        vec![json!({
            "id":"verify-query-selection",
            "code":"VERIFY_QUERY_SELECTION",
            "subject":query_context.requested_terms,
            "requiredCheckSet":["confirm exact declarations, callers, boundaries, and tests before publication"],
            "publicationBlocking":true,
        })]
    } else {
        Vec::new()
    };
    let status = if query_context.facts.is_empty() {
        "INCOMPLETE"
    } else if verified {
        "COMPLETE_TASK"
    } else {
        "CONDITIONAL_TASK"
    };
    let certainty = if verified { "VERIFIED" } else { "UNSURE" };
    let context = json!({
        "schema":"codeclew-bounded-context/2.0",
        "snapshot":{
            "baseRevision":session.base_revision,
            "snapshotId":snapshot.snapshot_id,
            "repositorySnapshot":ready.repository_snapshot,
            "generation":ready.generation,
            "queryIndex":ready.query_index,
        },
        "task":{"intent":intent,"terms":query_context.requested_terms},
        "compilation":session.compilation,
        "compilerVersion":ready.compiler_version,
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
            "mode":"STRICT",
            "status":if verified { "READY" } else { "BLOCKED_UNTIL_DISCHARGED" },
            "automaticPublication":verified,
        },
    });
    let projection = bounded_projection(&context)?;
    let evidence = json!({
        "schema":"codeclew-bounded-context-evidence/2.0",
        "context":context,
        "queryContext":query_context,
        "stdoutCompleteness":{
            "status":status,
            "certainty":certainty,
        },
    });
    Ok((projection, evidence))
}

fn load_fact_evidence(store: &CasStore, facts: &[FactHit]) -> Result<Vec<Value>, ClewError> {
    facts
        .iter()
        .map(|fact| {
            let limit = usize::try_from(fact.payload.size)
                .map_err(|_| resource("fact payload exceeds host size"))?
                .min(MAX_PAYLOAD_BYTES);
            if fact.payload.size > limit as u64 {
                return Err(resource("fact payload exceeds context limit"));
            }
            let lease = store.read(&fact.payload, limit)?;
            let payload = serde_json::from_slice::<Value>(lease.bytes()).unwrap_or_else(
                |_| json!({"opaquePayloadDigest":fact.payload.digest,"size":fact.payload.size}),
            );
            Ok(json!({
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
    paths: &BTreeSet<String>,
    terms: &[String],
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
        let (start_line, end_line, text) = snippet(source, terms);
        snippets.push(json!({
            "fileId":path,
            "contentRef":content,
            "startLine":start_line,
            "endLine":end_line,
            "text":text,
            "completeFile":text.len() == source.len(),
        }));
    }
    Ok(snippets)
}

fn snippet(source: &str, terms: &[String]) -> (usize, usize, String) {
    if source.len() <= MAX_SNIPPET_BYTES {
        return (1, source.lines().count().max(1), source.to_owned());
    }
    let lowered_terms = terms
        .iter()
        .map(|term| term.to_lowercase())
        .collect::<Vec<_>>();
    let lines = source.lines().collect::<Vec<_>>();
    let hit = lines
        .iter()
        .position(|line| {
            let line = line.to_lowercase();
            lowered_terms.iter().any(|term| line.contains(term))
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
        "schema":"codeclew-bounded-context-projection/2.0",
        "snapshot":context["snapshot"],
        "task":context["task"],
        "compilation":context["compilation"],
        "compilerVersion":context["compilerVersion"],
        "matches":[],
        "sources":[],
        "completeness":context["completeness"],
        "verificationObligations":context["verificationObligations"],
    });
    for key in ["sources", "matches"] {
        for value in context[key].as_array().into_iter().flatten() {
            projection[key]
                .as_array_mut()
                .expect("known array")
                .push(value.clone());
            if canonical::bytes(&projection).map_err(internal)?.len() > PROJECTION_TARGET_BYTES {
                projection[key].as_array_mut().expect("known array").pop();
                projection["truncated"] = Value::Bool(true);
                break;
            }
        }
    }
    if projection.get("truncated").is_none() {
        projection["truncated"] = Value::Bool(false);
    }
    Ok(projection)
}

fn rank_fact_evidence(mut facts: Vec<Value>, terms: &[String], limit: usize) -> Vec<Value> {
    let lowered = terms
        .iter()
        .map(|term| term.to_lowercase())
        .collect::<Vec<_>>();
    facts.sort_by(|left, right| {
        let score = |value: &Value| fact_score(&value["payload"], &lowered, None, 0);
        score(right)
            .cmp(&score(left))
            .then_with(|| left["factKey"].as_str().cmp(&right["factKey"].as_str()))
    });
    facts.truncate(limit.max(1));
    facts
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
