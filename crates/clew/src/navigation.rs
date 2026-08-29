use crate::canonical;
use crate::error::{ClewError, ErrorCode};
use crate::session::ContextObject;
use serde_json::{Map, Value, json};

pub const NAV_QUERY_SCHEMA: &str = "codeclew-nav-query/1.0";
pub const NAV_RESULT_SCHEMA: &str = "codeclew-navigation-result/1.0";
pub const NAV_QUERY_INTENT: &str = "NAVIGATION_QUERY";
pub const MAX_NAV_STDOUT_BYTES: usize = 64 * 1024;

pub fn query(context: &ContextObject) -> Result<Value, ClewError> {
    assemble(
        &context.session_id,
        &context.context_id,
        &context.intent,
        &context.terms,
        &context.projection,
    )
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
    for matched in matches {
        let Some(payload) = matched.get("payload").and_then(Value::as_object) else {
            continue;
        };
        if !is_declaration(payload) {
            continue;
        }
        let fact_key = required_string(matched, "factKey")?;
        let file = payload.get("file").and_then(Value::as_str);
        let source = file.and_then(|file| best_source(sources, file, payload));
        candidates.push(json!({
            "candidateId":fact_key,
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
            "callers":unsupported("RESOLVED_RELATION_FACTS_NOT_REQUESTED_BY_NAV_QUERY_V1"),
            "callees":unsupported("RESOLVED_RELATION_FACTS_NOT_REQUESTED_BY_NAV_QUERY_V1"),
            "tests":unsupported("TEST_RELATION_FACTS_NOT_REQUESTED_BY_NAV_QUERY_V1"),
        },
        "completeness":projection.get("completeness"),
        "truncated":projection.get("truncated").and_then(Value::as_bool).unwrap_or(false),
    });
    validate_stdout(&result)?;
    Ok(result)
}

fn is_declaration(payload: &Map<String, Value>) -> bool {
    payload
        .get("kind")
        .and_then(Value::as_str)
        .is_some_and(|kind| kind.eq_ignore_ascii_case("declaration"))
        || (payload.contains_key("declarationKind") && payload.contains_key("symbolIdentity"))
}

fn display_name<'a>(payload: &'a Map<String, Value>) -> Option<&'a str> {
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
        )
        .unwrap();
        assert_eq!(result["candidates"].as_array().unwrap().len(), 1);
        assert_eq!(result["candidates"][0]["candidateId"], "fact:target");
        assert_eq!(result["candidates"][0]["source"]["text"], "bounded");
        assert_eq!(result["facets"]["callers"]["status"], "UNSUPPORTED");
        assert_eq!(result["intent"], NAV_QUERY_INTENT);
    }

    #[test]
    fn accepts_compiler_declaration_shape_without_guessing_a_name() {
        let projection = json!({
            "matches":[{
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
        )
        .unwrap();
        assert_eq!(result["candidates"][0]["displayName"], "pkg.Target#run()V");
        assert_eq!(result["candidates"][0]["location"]["start"], 100);
    }

    #[test]
    fn refuses_navigation_stdout_above_the_public_bound() {
        let value = json!({"text":"x".repeat(MAX_NAV_STDOUT_BYTES)});
        let error = validate_stdout(&value).unwrap_err();
        assert_eq!(error.code, ErrorCode::SliceBudgetExceeded);
    }
}
