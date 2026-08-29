use crate::canonical;
use crate::error::{ClewError, ErrorCode};
use crate::session::ContextObject;
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub const NAV_QUERY_SCHEMA: &str = "codeclew-nav-query/1.0";
pub const NAV_EXPAND_SCHEMA: &str = "codeclew-nav-expand/2.0";
pub const NAV_RESULT_SCHEMA: &str = "codeclew-navigation-result/1.0";
pub const NAV_DELTA_SCHEMA: &str = "codeclew-navigation-delta/1.0";
pub const NAV_QUERY_INTENT: &str = "NAVIGATION_QUERY";
pub const MAX_NAV_STDOUT_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum NavigationFacet {
    Callers,
    Callees,
    Tests,
}

pub fn query(context: &ContextObject, facets: &[NavigationFacet]) -> Result<Value, ClewError> {
    let result = assemble(
        &context.session_id,
        &context.context_id,
        &context.intent,
        &context.terms,
        &context.projection,
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
        &parent.intent,
        &parent.terms,
        &parent.projection,
        &[],
    )?;
    let child_view = assemble(
        &child.session_id,
        &child.context_id,
        &child.intent,
        &child.terms,
        &child.projection,
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
        "facets":child_view["facets"],
        "completeness":child_view["completeness"],
        "truncated":child_view["truncated"],
    });
    validate_stdout(&result)?;
    Ok(result)
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
    intent: &str,
    terms: &[String],
    projection: &Value,
    requested_facets: &[NavigationFacet],
) -> Result<Value, ClewError> {
    let matches = projection
        .get("matches")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("navigation context has no match array"))?;
    let sources = projection
        .get("sources")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("navigation context has no source array"))?;

    let mut candidates = Vec::new();
    let mut candidate_identities = BTreeSet::new();
    for matched in matches {
        let Some(payload) = matched.get("payload").and_then(Value::as_object) else {
            continue;
        };
        if !is_declaration(payload) {
            continue;
        }
        let fact_key = required_string(matched, "factKey")?;
        let compilation = required_string(matched, "compilation")?;
        if let Some(identity) = payload.get("symbolIdentity").and_then(Value::as_str) {
            candidate_identities.insert(identity);
        }
        let file = payload.get("file").and_then(Value::as_str);
        let source = file.and_then(|file| best_source(sources, file, payload));
        candidates.push(json!({
            "candidateId":fact_key,
            "candidateKey":{"compilation":compilation,"factKey":fact_key},
            "displayName":display_name(payload).unwrap_or(fact_key),
            "declarationKind":payload.get("declarationKind"),
            "symbolIdentity":payload.get("symbolIdentity"),
            "location":{
                "file":file,
                "startLine":payload.get("startLine"),
                "endLine":payload.get("endLine"),
                "start":payload.get("start").or_else(|| payload.get("rangeStart")),
                "end":payload.get("end").or_else(|| payload.get("rangeEnd")),
            },
            "fact":{
                "factKey":fact_key,
                "domainUri":matched.get("domainUri"),
                "payloadRef":matched.get("payloadRef"),
                "payload":payload,
            },
            "source":source,
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
        "intent":intent,
        "terms":terms,
        "candidates":candidates,
        "facets":{
            "declaration":supported("RETAINED_DECLARATION_FACT"),
            "source":supported("BOUNDED_SESSION_SOURCE"),
            "callers":callers,
            "callees":callees,
            "tests":tests,
        },
        "completeness":projection.get("completeness"),
        "truncated":projection.get("truncated").and_then(Value::as_bool).unwrap_or(false),
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

fn best_source<'a>(
    sources: &'a [Value],
    file: &str,
    payload: &Map<String, Value>,
) -> Option<&'a Value> {
    let declaration_start = payload.get("startLine").and_then(Value::as_u64);
    let declaration_end = payload.get("endLine").and_then(Value::as_u64);
    sources
        .iter()
        .filter(|source| source.get("fileId").and_then(Value::as_str) == Some(file))
        .min_by_key(|source| {
            let start = source.get("startLine").and_then(Value::as_u64).unwrap_or(0);
            let end = source
                .get("endLine")
                .and_then(Value::as_u64)
                .unwrap_or(u64::MAX);
            let contains = declaration_start.is_none_or(|line| start <= line)
                && declaration_end.is_none_or(|line| line <= end);
            (!contains, end.saturating_sub(start), start)
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
                {"fileId":"src/lib.rs","startLine":1,"endLine":40,"text":"wide"},
                {"fileId":"src/lib.rs","startLine":8,"endLine":16,"text":"bounded"}
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
            NAV_QUERY_INTENT,
            &["Target".into()],
            &projection(),
            &[],
        )
        .unwrap();
        assert_eq!(result["candidates"].as_array().unwrap().len(), 1);
        assert_eq!(result["candidates"][0]["candidateId"], "fact:target");
        assert_eq!(result["candidates"][0]["source"]["text"], "bounded");
        assert_eq!(result["facets"]["callers"]["status"], "NOT_REQUESTED");
        assert_eq!(result["intent"], NAV_QUERY_INTENT);
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
            NAV_QUERY_INTENT,
            &["run".into()],
            &projection,
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
            NAV_QUERY_INTENT,
            &["Target".into()],
            &projection,
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
            evidence: json!({}),
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
