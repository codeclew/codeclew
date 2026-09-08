//! Spring trigger metadata over resolved compiler declarations. Runtime activation
//! and transport handoffs are deliberately separate from compiler authority.
use crate::canonical;
use crate::cas::CasStore;
use crate::error::{ClewError, ErrorCode};
use crate::generation_service::load_session_generation;
use crate::generation_v2::GenerationManifest;
use crate::session::{SessionAuthority, SessionLanguage};
use crate::state::StateAuthority;
use serde_json::{Value, json};
use std::collections::BTreeSet;

const MAX_PAYLOAD: usize = 2 * 1024 * 1024;
const MAX_CATALOGUE: usize = 64 * 1024 * 1024;
const MAX_STDOUT: usize = 64 * 1024;

pub use clew_framework_spring::{SpringEntry, SpringMetadata, describe_trigger};

fn invalid(message: impl Into<String>) -> ClewError {
    ClewError::new(ErrorCode::InvalidInput, message)
}

/// Derive from the retained portable compiler facts. Legacy metadata remains a
/// compatibility input for already sealed generations; new workers emit facts.
pub fn metadata_for_fact(
    fact: &Value,
    authority: &str,
) -> Result<Option<SpringMetadata>, ClewError> {
    if let Some(input) = fact.get("jvmAnnotations") {
        validate_annotation_facts(
            input,
            fact.get("symbolIdentity").and_then(Value::as_str),
            authority,
        )?;
        let input: clew_facts::JvmAnnotationFacts = serde_json::from_value(input.clone())
            .map_err(|_| invalid("JVM annotation facts violate their closed contract"))?;
        return clew_framework_spring::analyze(&input)
            .map(Some)
            .map_err(invalid);
    }
    fact.get("spring")
        .map(|value| validate_metadata(value, authority))
        .transpose()
}

pub fn validate_annotation_facts(
    value: &Value,
    declaration: Option<&str>,
    authority: &str,
) -> Result<(), ClewError> {
    let input: clew_facts::JvmAnnotationFacts = serde_json::from_value(value.clone())
        .map_err(|_| invalid("JVM annotation facts violate their closed contract"))?;
    input.validate().map_err(invalid)?;
    if input.authority != authority || Some(input.declaration.as_str()) != declaration {
        return Err(invalid(
            "JVM annotation facts differ from their compiler declaration authority",
        ));
    }
    Ok(())
}

pub fn validate_metadata(value: &Value, authority: &str) -> Result<SpringMetadata, ClewError> {
    let metadata: SpringMetadata = serde_json::from_value(value.clone())
        .map_err(|_| invalid("Spring metadata violates its closed contract"))?;
    if metadata.schema != "spring-entrypoints/0.1"
        || metadata.authority != authority
        || metadata.derivation.is_some()
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
                    scope_boundaries.insert(payload.get("code").and_then(Value::as_str).unwrap_or("JAVA_ANALYSIS_BOUNDARY").to_owned());
                }
                if !matches!(payload.get("declarationKind").and_then(Value::as_str), Some("FUNCTION" | "METHOD" | "CLASS")) { return Ok(()); }
                descriptors += 1;
                if kotlin { crate::semantic_validation::validate_declaration_descriptor_fact(&payload)?; }
                let Some(metadata) = metadata_for_fact(&payload, if kotlin { "K2_RESOLVED_ANNOTATIONS" } else { "JAVAC_RESOLVED_ANNOTATIONS" })? else { return Ok(()); };
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
                        "annotationAuthority":metadata.derivation.as_ref().map(|derivation| derivation.input_authority.as_str()).unwrap_or(&metadata.authority),
                        "frameworkDerivation":metadata.derivation,"runtimeActivation":"UNPROVEN"
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

    #[test]
    fn portable_annotation_authority_is_bound_and_derivation_tracks_partial_inputs() {
        let mut fact = json!({"symbolIdentity":"class:example/Service", "jvmAnnotations":{
            "schema":"jvm-annotation-facts/1.0", "authority":"K2_RESOLVED_ANNOTATIONS",
            "declaration":"class:example/Service", "definitions":{}, "types":[], "callables":[],
            "boundaries":[], "coverage":{"status":"COMPLETE","scope":"REACHABLE_ANNOTATIONS_AND_HIERARCHY"}
        }});
        let complete = metadata_for_fact(&fact, "K2_RESOLVED_ANNOTATIONS")
            .unwrap()
            .unwrap();
        assert_eq!(complete.authority, "FRAMEWORK_DERIVED");
        assert_eq!(complete.derivation.as_ref().unwrap().coverage, "COMPLETE");
        assert!(metadata_for_fact(&fact, "JAVAC_RESOLVED_ANNOTATIONS").is_err());
        fact["jvmAnnotations"]["declaration"] = json!("class:example/Other");
        assert!(metadata_for_fact(&fact, "K2_RESOLVED_ANNOTATIONS").is_err());
        fact["jvmAnnotations"]["declaration"] = fact["symbolIdentity"].clone();
        fact["jvmAnnotations"]["boundaries"] = json!(["ANNOTATION_DECLARATION_UNAVAILABLE"]);
        assert!(metadata_for_fact(&fact, "K2_RESOLVED_ANNOTATIONS").is_err());
        fact["jvmAnnotations"]["coverage"]["status"] = json!("PARTIAL");
        let partial = metadata_for_fact(&fact, "K2_RESOLVED_ANNOTATIONS")
            .unwrap()
            .unwrap();
        assert_eq!(partial.derivation.as_ref().unwrap().coverage, "PARTIAL");
        assert_ne!(
            complete.derivation.unwrap().input_digest,
            partial.derivation.unwrap().input_digest
        );
        assert_eq!(
            partial.boundaries,
            vec!["ANNOTATION_DECLARATION_UNAVAILABLE"]
        );
        fact.as_object_mut().unwrap().remove("jvmAnnotations");
        assert!(
            metadata_for_fact(&fact, "K2_RESOLVED_ANNOTATIONS")
                .unwrap()
                .is_none()
        );
    }
}
