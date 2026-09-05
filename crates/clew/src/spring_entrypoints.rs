//! Spring trigger metadata over resolved compiler declarations. Runtime activation
//! and transport handoffs are deliberately separate from compiler authority.
use crate::canonical;
use crate::cas::CasStore;
use crate::error::{ClewError, ErrorCode};
use crate::generation_service::load_session_generation;
use crate::generation_v2::GenerationManifest;
use crate::session::{SessionAuthority, SessionLanguage};
use crate::state::StateAuthority;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;

const MAX_PAYLOAD: usize = 2 * 1024 * 1024;
const MAX_CATALOGUE: usize = 64 * 1024 * 1024;
const MAX_STDOUT: usize = 64 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpringMetadata {
    pub schema: String,
    pub authority: String,
    pub entries: Vec<SpringEntry>,
    pub boundaries: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpringEntry {
    pub kind: String,
    pub annotation: String,
    pub annotation_chain: Vec<String>,
    pub attributes: serde_json::Map<String, Value>,
    pub registration: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub controller: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class_attributes: Option<Vec<serde_json::Map<String, Value>>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handler_attributes: Option<serde_json::Map<String, Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_symbol: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bean_class: Option<String>,
}

fn invalid(message: impl Into<String>) -> ClewError {
    ClewError::new(ErrorCode::InvalidInput, message)
}

pub fn validate_metadata(value: &Value, authority: &str) -> Result<SpringMetadata, ClewError> {
    let metadata: SpringMetadata = serde_json::from_value(value.clone())
        .map_err(|_| invalid("Spring metadata violates its closed contract"))?;
    if metadata.schema != "spring-entrypoints/0.1"
        || metadata.authority != authority
        || metadata.entries.len() > 2048
        || metadata.boundaries.len() > 256
        || metadata
            .boundaries
            .iter()
            .any(|code| code.is_empty() || code.len() > 128)
    {
        return Err(invalid("Spring metadata authority or size is invalid"));
    }
    for entry in &metadata.entries {
        let annotation = match entry.kind.as_str() {
            "HTTP_ENDPOINT" => "org.springframework.web.bind.annotation.RequestMapping",
            "KAFKA_LISTENER" => "org.springframework.kafka.annotation.KafkaListener",
            "SCHEDULED_JOB" => "org.springframework.scheduling.annotation.Scheduled",
            _ => return Err(invalid("unknown Spring entrypoint kind")),
        };
        if entry.annotation != annotation
            || entry.registration != "RUNTIME_CONDITIONAL"
            || entry.annotation_chain.is_empty()
            || entry.annotation_chain.len() > 34
            || entry
                .annotation_chain
                .iter()
                .any(|name| name.is_empty() || name.len() > 1024)
            || (entry.kind == "HTTP_ENDPOINT") != entry.controller.is_some()
            || (entry.kind == "HTTP_ENDPOINT") != entry.class_attributes.is_some()
            || entry.kind != "KAFKA_LISTENER" && entry.handler_attributes.is_some()
        {
            return Err(invalid(
                "Spring trigger identity or registration authority is invalid",
            ));
        }
    }
    Ok(metadata)
}

fn strings(value: Option<&Value>) -> Option<Vec<String>> {
    match value {
        None => Some(Vec::new()),
        Some(Value::String(value)) => Some(vec![value.clone()]),
        Some(Value::Array(values)) => values
            .iter()
            .map(|value| value.as_str().map(str::to_owned))
            .collect(),
        _ => None,
    }
}

fn combine_path(parent: &str, child: &str) -> Option<String> {
    if [parent, child]
        .iter()
        .any(|value| value.contains("${") || value.contains("#{"))
    {
        return None;
    }
    // Spring removes a terminal single-segment wildcard when combining paths.
    // Other wildcard combinations need PathPattern/AntPathMatcher configuration.
    let parent = if !child.is_empty() {
        parent.strip_suffix("/*").unwrap_or(parent)
    } else {
        parent
    };
    if parent.contains('*') || parent.contains('?') {
        return None;
    }
    Some(match (parent.is_empty(), child.is_empty()) {
        (true, true) => String::new(),
        (true, false) => {
            if child.starts_with('/') {
                child.to_owned()
            } else {
                format!("/{child}")
            }
        }
        (false, true) => {
            if parent.starts_with('/') {
                parent.to_owned()
            } else {
                format!("/{parent}")
            }
        }
        (false, false) => format!(
            "{}/{}",
            if parent.starts_with('/') {
                parent.to_owned()
            } else {
                format!("/{parent}")
            }
            .trim_end_matches('/'),
            child.trim_start_matches('/')
        ),
    })
}

fn contains_runtime_value(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(value) => value.contains("${") || value.contains("#{"),
        Value::Array(values) => values.iter().any(contains_runtime_value),
        Value::Object(values) => values.values().any(contains_runtime_value),
        _ => false,
    }
}

/// Preserve raw attributes as evidence; this projection is only a convenient
/// Spring-rule-derived description, never evidence that a live bean was registered.
pub fn describe_trigger(entry: &SpringEntry) -> Value {
    let attributes = &entry.attributes;
    match entry.kind.as_str() {
        "HTTP_ENDPOINT" => {
            let empty = serde_json::Map::new();
            let class = entry
                .class_attributes
                .as_ref()
                .and_then(|list| list.first())
                .unwrap_or(&empty);
            let methods = strings(attributes.get("method")).and_then(|method| {
                let class = strings(class.get("method"))?;
                let result: BTreeSet<_> = class.into_iter().chain(method).collect();
                Some(if result.is_empty() {
                    vec!["ANY".to_owned()]
                } else {
                    result.into_iter().collect()
                })
            });
            let paths =
                strings(class.get("path").or(class.get("value"))).and_then(|mut parents| {
                    let mut children = strings(attributes.get("path").or(attributes.get("value")))?;
                    if parents.is_empty() {
                        parents.push(String::new());
                    }
                    if children.is_empty() {
                        children.push(String::new());
                    }
                    if parents.len().checked_mul(children.len())? > 4096 {
                        return None;
                    }
                    let combined = parents
                        .iter()
                        .flat_map(|parent| {
                            children
                                .iter()
                                .map(move |child| combine_path(parent, child))
                        })
                        .collect::<Option<BTreeSet<_>>>()?;
                    Some(combined.into_iter().collect::<Vec<_>>())
                });
            let mut conditions = serde_json::Map::new();
            for key in ["params", "headers"] {
                conditions.insert(
                    key.into(),
                    json!(strings(class.get(key)).and_then(|class| {
                        Some(
                            class
                                .into_iter()
                                .chain(strings(attributes.get(key))?)
                                .collect::<Vec<_>>(),
                        )
                    })),
                );
            }
            for key in ["consumes", "produces"] {
                let method = strings(attributes.get(key));
                conditions.insert(
                    key.into(),
                    json!(match method {
                        Some(ref value) if value.is_empty() => strings(class.get(key)),
                        value => value,
                    }),
                );
            }
            json!({"pathResolution":if paths.is_some(){"DERIVED"}else{"REQUIRES_RUNTIME_OR_PATH_PATTERN_RESOLUTION"},
                "paths":paths,"methods":methods,"conditions":conditions,"authority":"SPRING_ANNOTATION_RULES"})
        }
        "SCHEDULED_JOB" => json!({
            "disabled":if attributes.values().any(contains_runtime_value) {None} else {Some(attributes.get("cron").and_then(Value::as_str) == Some("-"))},
            "timeUnit":attributes.get("timeUnit").cloned().unwrap_or(json!("MILLISECONDS")),
            "configuration":attributes,
            "authority":"SPRING_ANNOTATION_RULES"
        }),
        _ => json!({"configuration":attributes,"authority":"SPRING_ANNOTATION_RULES"}),
    }
}

/// Read every descriptor in the explicitly bound generations, not a lexical
/// query prefix. A cursor binds pagination to the complete immutable catalogue.
pub fn thread_catalogue(
    thread: &crate::thread::ThreadAuthority,
    cursor: Option<&str>,
    limit: usize,
) -> Result<Value, ClewError> {
    let state = StateAuthority::process_default()?;
    let _admission = thread.admit_with_state(&state)?;
    catalogue(
        thread
            .members
            .iter()
            .map(|member| (member.member_alias.clone(), member.session.clone()))
            .collect(),
        cursor,
        limit,
    )
}

pub fn catalogue(
    mut sessions: Vec<(String, SessionAuthority)>,
    cursor: Option<&str>,
    limit: usize,
) -> Result<Value, ClewError> {
    if sessions.is_empty() || sessions.len() > 64 || !(1..=100).contains(&limit) {
        return Err(invalid(
            "entrypoints requires 1..64 sessions and limit 1..100",
        ));
    }
    let state = StateAuthority::process_default()?;
    let store = CasStore::open(&state)?;
    let mut admissions = Vec::new();
    let mut roots = Vec::new();
    let mut scopes = Vec::new();
    let mut seen_sessions = BTreeSet::new();
    let mut bytes = 0usize;
    // Lock in stable order, including requests supplied in reverse CLI order.
    sessions.sort_by(|a, b| a.1.session_id.cmp(&b.1.session_id));
    for (member, session) in sessions {
        if !seen_sessions.insert(session.session_id.clone()) {
            return Err(invalid("duplicate entrypoint session"));
        }
        if !matches!(
            session.language,
            SessionLanguage::Kotlin | SessionLanguage::Java
        ) {
            return Err(invalid("entrypoints supports Kotlin and Java sessions"));
        }
        admissions.push(session.open_admission()?);
        let ready = load_session_generation(&session)?;
        for compilation in &ready.compilations {
            let lease = store.read(&compilation.generation, MAX_CATALOGUE)?;
            let generation: GenerationManifest = serde_json::from_slice(lease.bytes())
                .map_err(|_| invalid("entrypoint generation manifest is invalid"))?;
            if generation.derived_input_manifest != compilation.derived_input_manifest
                || canonical::bytes(&generation).map_err(|error| invalid(error.to_string()))?
                    != lease.bytes()
            {
                return Err(invalid("entrypoint generation has inconsistent authority"));
            }
            let mut descriptors = 0usize;
            let mut inspected = 0usize;
            let mut scope_boundaries = BTreeSet::new();
            generation.visit_facts(&store, |fact| {
                let kotlin = fact.fact_key.starts_with("kotlin:descriptor:");
                let java = fact.domain_uri.as_str() == "analysis:java-compiler-facts";
                if !kotlin && !java {
                    if fact.fact_key.starts_with("kotlin:metadata:") {
                        let lease = store.read(&fact.payload, MAX_PAYLOAD)?;
                        let metadata: Value = serde_json::from_slice(lease.bytes()).map_err(|_| invalid("Kotlin index metadata is invalid"))?;
                        if let Some(boundaries) = metadata.get("buildModelBoundaries").and_then(Value::as_array) {
                            scope_boundaries.extend(boundaries.iter().filter_map(Value::as_str).map(str::to_owned));
                        }
                    }
                    if fact.fact_key.starts_with("kotlin:descriptor-boundary:") { scope_boundaries.insert("DESCRIPTOR_COVERAGE_PARTIAL".to_owned()); }
                    return Ok(());
                }
                let lease = store.read(&fact.payload, MAX_PAYLOAD)?;
                let payload: Value = serde_json::from_slice(lease.bytes()).map_err(|_| invalid("entrypoint fact is invalid"))?;
                if java && payload.get("kind").and_then(Value::as_str) == Some("BOUNDARY") {
                    scope_boundaries.insert("JAVA_ANALYSIS_BOUNDARY".to_owned());
                }
                if !matches!(payload.get("declarationKind").and_then(Value::as_str), Some("FUNCTION" | "METHOD" | "CLASS")) { return Ok(()); }
                descriptors += 1;
                let Some(spring) = payload.get("spring") else { return Ok(()); };
                if kotlin { crate::semantic_validation::validate_declaration_descriptor_fact(&payload)?; }
                let metadata = validate_metadata(spring, if kotlin { "K2_RESOLVED_ANNOTATIONS" } else { "JAVAC_RESOLVED_ANNOTATIONS" })?;
                inspected += 1;
                scope_boundaries.extend(metadata.boundaries.iter().cloned());
                for (ordinal, entry) in metadata.entries.iter().enumerate() {
                    let identity = json!({"repository":session.repository_key,"revision":session.base_revision,
                        "compilation":compilation.compilation,"member":member,"symbol":payload.get("symbolIdentity"),"ordinal":ordinal});
                    let root = json!({
                        "id":canonical::hash(&identity).map_err(|error| invalid(error.to_string()))?,
                        "member":member,"repositoryKey":session.repository_key,"baseRevision":session.base_revision,
                        "sessionId":session.session_id,"compilation":compilation.compilation,
                        "language":session.language.uri(),"symbolIdentity":entry.target_symbol.as_ref().map(|s|json!(s)).unwrap_or_else(||payload["symbolIdentity"].clone()),
                        "ownerIdentity":payload.get("ownerIdentity"),"kind":entry.kind,
                        "file":payload.get("file"),"start":payload.get("start"),"end":payload.get("end"),
                        "coordinateUnit":if kotlin {"UTF8_BYTES"}else{"UTF16_CODE_UNITS"},
                        "startLine":payload.get("startLine"),"endLine":payload.get("endLine"),
                        "trigger":describe_trigger(entry),"binding":entry,"boundaries":metadata.boundaries,
                        "factKey":fact.fact_key,"evidence":fact.payload,"generation":compilation.generation,
                        "annotationAuthority":metadata.authority,"runtimeActivation":"UNPROVEN"
                    });
                    bytes = bytes.checked_add(canonical::bytes(&root).map_err(|error| invalid(error.to_string()))?.len())
                        .ok_or_else(|| invalid("entrypoint catalogue size overflow"))?;
                    if bytes > MAX_CATALOGUE { return Err(ClewError::new(ErrorCode::SliceBudgetExceeded,"entrypoint catalogue exceeds 64 MiB; select fewer sessions or compilations")); }
                    roots.push(root);
                }
                Ok(())
            })?;
            if inspected != descriptors {
                scope_boundaries
                    .insert("SPRING_EXTRACTION_UNAVAILABLE_FOR_SOME_DECLARATIONS".into());
            }
            if descriptors == 0 {
                scope_boundaries
                    .insert("EMPTY_DECLARATION_SCOPE_REQUIRES_EXTRACTION_COVERAGE".into());
            }
            scopes.push(json!({"member":member,"sessionId":session.session_id,"repositoryKey":session.repository_key,
                "baseRevision":session.base_revision,"compilation":compilation.compilation,"generation":compilation.generation,
                "declarations":descriptors,"inspectedDeclarations":inspected,"boundaries":scope_boundaries,
                "generationCoverage":compilation.coverage,"generationCertainty":compilation.certainty,
                "generationObligations":compilation.obligations}));
        }
    }
    roots.sort_by_cached_key(|root| root["id"].as_str().unwrap_or_default().to_owned());
    scopes.sort_by_cached_key(Value::to_string);
    let digest = canonical::hash(&json!({"roots":roots,"scopes":scopes}))
        .map_err(|error| invalid(error.to_string()))?;
    catalogue_page(&roots, &scopes, &digest, cursor, limit)
}

fn catalogue_page(
    roots: &[Value],
    scopes: &[Value],
    digest: &str,
    cursor: Option<&str>,
    limit: usize,
) -> Result<Value, ClewError> {
    let offset = if let Some(cursor) = cursor {
        let (expected, offset) = cursor
            .rsplit_once('@')
            .ok_or_else(|| invalid("invalid entrypoint cursor"))?;
        if expected != digest {
            return Err(invalid("entrypoint cursor belongs to another catalogue"));
        }
        offset
            .parse::<usize>()
            .map_err(|_| invalid("invalid entrypoint cursor offset"))?
    } else {
        0
    };
    if offset > roots.len() {
        return Err(invalid("entrypoint cursor exceeds catalogue"));
    }
    let mut end = (offset + limit).min(roots.len());
    loop {
        let result = json!({"schema":"codeclew-entrypoints/1.0","catalogueDigest":digest,
            "total":roots.len(),"offset":offset,"entries":&roots[offset..end],"scopes":scopes,
            "nextCursor":if end < roots.len() {Some(format!("{digest}@{end}"))}else{None},
            "runtimeActivation":"UNPROVEN","scope":"ANNOTATION_DECLARED_COMPUTATION_ROOTS",
            "obligations":["VERIFY_BEAN_ACTIVATION_AND_RUNTIME_CONFIGURATION","VERIFY_PROGRAMMATIC_REGISTRATIONS"]});
        if canonical::bytes(&result)
            .map_err(|error| invalid(error.to_string()))?
            .len()
            <= MAX_STDOUT
        {
            return Ok(result);
        }
        if end <= offset + 1 {
            return Err(ClewError::new(
                ErrorCode::SliceBudgetExceeded,
                "entrypoint metadata exceeds stdout budget; select fewer sessions or compilations",
            ));
        }
        end -= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn http(class: Value, method: Value) -> SpringEntry {
        serde_json::from_value(json!({
            "kind":"HTTP_ENDPOINT","annotation":"org.springframework.web.bind.annotation.RequestMapping",
            "annotationChain":["org.springframework.web.bind.annotation.GetMapping"],
            "attributes":method,"classAttributes":[class],"controller":true,"registration":"RUNTIME_CONDITIONAL"
        })).unwrap()
    }

    #[test]
    fn http_preserves_spring_combination_conditions_and_unrestricted_methods() {
        let entry = http(
            json!({"path":["/api","/v2"],"headers":["X-Tenant"],"consumes":["application/xml"]}),
            json!({"path":["/items","/products"],"params":["active=true"],"consumes":["application/json"]}),
        );
        let trigger = describe_trigger(&entry);
        assert_eq!(
            trigger["paths"],
            json!(["/api/items", "/api/products", "/v2/items", "/v2/products"])
        );
        assert_eq!(trigger["methods"], json!(["ANY"]));
        assert_eq!(trigger["conditions"]["headers"], json!(["X-Tenant"]));
        assert_eq!(
            trigger["conditions"]["consumes"],
            json!(["application/json"])
        );
        assert_eq!(
            describe_trigger(&http(
                json!({"path":["/hotels/*"]}),
                json!({"path":["/booking"]})
            ))["paths"],
            json!(["/hotels/booking"])
        );
        let unknown = describe_trigger(&http(
            json!({"path":["/hotels/**"]}),
            json!({"path":["/booking"]}),
        ));
        assert!(unknown["paths"].is_null());
        assert_eq!(
            unknown["pathResolution"],
            "REQUIRES_RUNTIME_OR_PATH_PATTERN_RESOLUTION"
        );
        assert!(
            describe_trigger(&http(
                json!({"path":["${prefix}"]}),
                json!({"path":["/booking"]})
            ))["paths"]
                .is_null()
        );
    }

    #[test]
    fn schedule_disable_state_is_unknown_until_runtime_expressions_resolve() {
        let mut entry: SpringEntry = serde_json::from_value(json!({
            "kind":"SCHEDULED_JOB","annotation":"org.springframework.scheduling.annotation.Scheduled",
            "annotationChain":["org.springframework.scheduling.annotation.Scheduled"],
            "attributes":{"cron":"-"},"registration":"RUNTIME_CONDITIONAL"
        })).unwrap();
        assert_eq!(describe_trigger(&entry)["disabled"], true);
        entry
            .attributes
            .insert("cron".into(), json!("${job.cron:-}"));
        assert!(describe_trigger(&entry)["disabled"].is_null());
        entry.attributes.insert("cron".into(), Value::Null);
        assert!(describe_trigger(&entry)["disabled"].is_null());
    }

    #[test]
    fn catalogue_pagination_covers_every_root_and_rejects_stale_cursors() {
        let roots = (0..117)
            .map(|id| json!({"id":id,"member":if id%2==0{"a"}else{"b"}}))
            .collect::<Vec<_>>();
        let digest = canonical::hash(&roots).unwrap();
        let mut cursor = None;
        let mut all = Vec::new();
        loop {
            let page = catalogue_page(&roots, &[], &digest, cursor.as_deref(), 13).unwrap();
            assert_eq!(page["total"], 117);
            all.extend(page["entries"].as_array().unwrap().iter().cloned());
            cursor = page["nextCursor"].as_str().map(str::to_owned);
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(roots, all);
        assert!(catalogue_page(&roots, &[], &digest, Some("wrong@13"), 13).is_err());
        assert!(catalogue_page(&roots, &[], &digest, Some(&format!("{digest}@118")), 13).is_err());
        let large = (0..10)
            .map(|id| json!({"id":id,"metadata":"x".repeat(12000)}))
            .collect::<Vec<_>>();
        let page = catalogue_page(&large, &[], "digest", None, 100).unwrap();
        assert!(canonical::bytes(&page).unwrap().len() <= MAX_STDOUT);
        assert!(page["nextCursor"].as_str().is_some());
        assert!(!page["entries"].as_array().unwrap().is_empty());
    }

    #[test]
    fn compiler_metadata_cannot_promote_runtime_authority_or_change_kind() {
        let mut value = json!({"schema":"spring-entrypoints/0.1","authority":"K2_RESOLVED_ANNOTATIONS",
            "entries":[http(json!({}),json!({}))],"boundaries":[]});
        assert!(validate_metadata(&value, "K2_RESOLVED_ANNOTATIONS").is_ok());
        value["entries"][0]["registration"] = json!("VERIFIED");
        assert!(validate_metadata(&value, "K2_RESOLVED_ANNOTATIONS").is_err());
        value["entries"][0]["registration"] = json!("RUNTIME_CONDITIONAL");
        value["entries"][0]["annotation"] = json!("impostor.GetMapping");
        assert!(validate_metadata(&value, "K2_RESOLVED_ANNOTATIONS").is_err());
    }
}
