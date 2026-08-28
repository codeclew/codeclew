use crate::canonical;
use crate::cas::{CasObject, CasStore};
use crate::derived_manifest::DerivedAnalysisInputManifest;
use crate::error::{ClewError, ErrorCode};
use crate::generation_service::{ReadyGeneration, ReadyGenerationSet, load_session_generation};
use crate::generation_v2::GenerationManifest;
use crate::java_adapter_v2::{JAVA_COMPILER_FACTS_CAPABILITY, JavaCompilerFact};
use crate::jvm_navigation::{
    JVM_NAVIGATION_SCHEMA, JvmDeclaration, JvmLanguage, JvmMemberMode, JvmNavigationBoundary,
    JvmNavigationMatch, JvmReference, JvmSourceAnchor, MAX_JVM_NAVIGATION_RESULTS,
    java_callable_family, kotlin_callable_family, kotlin_callable_identity, kotlin_class_identity,
    resolve,
};
use crate::kotlin_adapter_v2::KOTLIN_FACTS_CAPABILITY;
use crate::semantic_validation::{KotlinSemanticPayloadKind, validate_kotlin_semantic_payload};
use crate::session::{SessionAuthority, SessionLanguage};
use crate::state::StateAuthority;
use crate::thread::ThreadAuthority;
use crate::thread_callables_service::ThreadCallablesRequest;
use crate::thread_context::{MAX_THREAD_STDOUT_BYTES, ThreadContextObject};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::{Component, Path};

const MAX_VISITED_FACTS: usize = 131_072;
const MAX_PAYLOAD_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JvmNavigationGeneration {
    pub member_alias: String,
    pub language: JvmLanguage,
    pub compilation: String,
    pub generation: CasObject,
    pub semantic_input_authority_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JvmNavigationClaimBoundary {
    pub artifact_ownership: String,
    pub compatibility: String,
    pub framework_semantics: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JvmNavigationResult {
    pub schema: String,
    pub thread_id: String,
    pub thread_authority_digest: String,
    pub context_id: String,
    pub context_authority_digest: String,
    pub task_id: String,
    pub pair_id: String,
    pub provider_member: String,
    pub consumer_member: String,
    pub provider_language: JvmLanguage,
    pub consumer_language: JvmLanguage,
    pub terms: Vec<String>,
    pub generations: Vec<JvmNavigationGeneration>,
    pub matches: Vec<JvmNavigationMatch>,
    pub boundaries: Vec<JvmNavigationBoundary>,
    pub truncated: bool,
    pub claim_boundary: JvmNavigationClaimBoundary,
}

#[derive(Debug)]
struct CapturedFacts {
    language: JvmLanguage,
    generations: Vec<JvmNavigationGeneration>,
    declarations: Vec<JvmDeclaration>,
    references: Vec<JvmReference>,
    boundaries: Vec<JvmNavigationBoundary>,
}

pub fn create(
    thread: &ThreadAuthority,
    context_id: &str,
    request: ThreadCallablesRequest,
) -> Result<Value, ClewError> {
    thread.verify()?;
    let state = StateAuthority::process_default()?;
    thread.require_open_with_state(&state)?;
    let context = ThreadContextObject::load(thread, context_id)?;
    if context.authority.thread_id != thread.thread_id
        || context.authority.thread_authority_digest != thread.authority_digest
    {
        return Err(invalid(
            "thread context belongs to another thread authority",
        ));
    }
    validate_request(thread, &request)?;
    let store = CasStore::open(&state)?;
    let provider = capture_member(thread, &context, &store, &request.provider_member)?;
    let consumer = capture_member(thread, &context, &store, &request.consumer_member)?;
    if provider.language == consumer.language {
        return Err(invalid(
            "JVM convergence requires one Kotlin and one Java member",
        ));
    }
    let consumer_digest = combined_semantic_authority(&consumer.generations)?;
    let mut resolution = resolve(
        &consumer.references,
        &provider.declarations,
        &consumer_digest,
        &request.terms,
    );
    resolution.boundaries.extend(consumer.boundaries);
    resolution.boundaries.sort_by(|left, right| {
        (&left.source_identity, &left.raw_target, &left.code).cmp(&(
            &right.source_identity,
            &right.raw_target,
            &right.code,
        ))
    });
    resolution.boundaries.dedup();
    let available = MAX_JVM_NAVIGATION_RESULTS.saturating_sub(resolution.matches.len());
    if resolution.boundaries.len() > available {
        resolution.boundaries.truncate(available);
        resolution.truncated = true;
    }
    let mut generations = provider.generations;
    generations.extend(consumer.generations);
    generations.sort_by(|left, right| {
        (&left.member_alias, &left.compilation).cmp(&(&right.member_alias, &right.compilation))
    });
    let result = JvmNavigationResult {
        schema: JVM_NAVIGATION_SCHEMA.into(),
        thread_id: thread.thread_id.clone(),
        thread_authority_digest: thread.authority_digest.clone(),
        context_id: context.context_id.clone(),
        context_authority_digest: context.authority.authority_digest.clone(),
        task_id: request.task_id,
        pair_id: request.pair_id,
        provider_member: request.provider_member,
        consumer_member: request.consumer_member,
        provider_language: provider.language,
        consumer_language: consumer.language,
        terms: request.terms,
        generations,
        matches: resolution.matches,
        boundaries: resolution.boundaries,
        truncated: resolution.truncated,
        claim_boundary: JvmNavigationClaimBoundary {
            artifact_ownership: "UNVERIFIED".into(),
            compatibility: "NOT_ASSESSED".into(),
            framework_semantics: "NOT_ASSESSED".into(),
        },
    };
    let value = serde_json::to_value(result).map_err(internal)?;
    if canonical::bytes(&value)
        .map_err(internal)?
        .len()
        .saturating_add(1)
        > MAX_THREAD_STDOUT_BYTES
    {
        return Err(ClewError::new(
            ErrorCode::SliceBudgetExceeded,
            "JVM navigation stdout exceeds 64 KiB",
        ));
    }
    Ok(value)
}

fn capture_member(
    thread: &ThreadAuthority,
    context: &ThreadContextObject,
    store: &CasStore,
    alias: &str,
) -> Result<CapturedFacts, ClewError> {
    let binding = thread
        .members
        .iter()
        .find(|member| member.member_alias == alias)
        .ok_or_else(|| invalid("JVM navigation member is absent from the thread"))?;
    let context_binding = context
        .authority
        .members
        .iter()
        .find(|member| member.member_alias == alias)
        .ok_or_else(|| invalid("JVM navigation member is absent from the context"))?;
    let (session, _) = SessionAuthority::load(&binding.session.session_id)?;
    if canonical::bytes(&session).map_err(internal)?
        != canonical::bytes(&binding.session).map_err(internal)?
        || context_binding.session_id != session.session_id
        || context_binding.session_authority_digest != session.authority_digest
        || context_binding.language != session.language.uri()
    {
        return Err(ClewError::new(
            ErrorCode::BindingChanged,
            "JVM navigation member session or context authority changed",
        ));
    }
    session.require_open()?;
    let language = match session.language {
        SessionLanguage::Kotlin => JvmLanguage::Kotlin,
        SessionLanguage::Java => JvmLanguage::Java,
        _ => {
            return Err(ClewError::new(
                ErrorCode::UnsupportedLanguage,
                "JVM convergence supports only Kotlin and Java members",
            ));
        }
    };
    let ready = load_session_generation(&session)?;
    validate_generation_binding(&ready, &session, context_binding)?;
    let mut captured = CapturedFacts {
        language,
        generations: Vec::new(),
        declarations: Vec::new(),
        references: Vec::new(),
        boundaries: Vec::new(),
    };
    let mut visited = 0usize;
    let mut bytes = 0usize;
    for compilation in &ready.compilations {
        let semantic_digest = semantic_input_authority(store, compilation, language)?;
        captured.generations.push(JvmNavigationGeneration {
            member_alias: alias.into(),
            language,
            compilation: compilation.compilation.clone(),
            generation: compilation.generation.clone(),
            semantic_input_authority_digest: semantic_digest,
        });
        let generation: GenerationManifest = read_canonical(store, &compilation.generation)?;
        generation.visit_facts(store, |fact| {
            visited = visited
                .checked_add(1)
                .filter(|count| *count <= MAX_VISITED_FACTS)
                .ok_or_else(|| budget("JVM navigation visited fact budget exceeded"))?;
            let size = usize::try_from(fact.payload.size)
                .map_err(|_| budget("JVM navigation payload exceeds host size"))?;
            bytes = bytes
                .checked_add(size)
                .filter(|count| *count <= MAX_PAYLOAD_BYTES)
                .ok_or_else(|| budget("JVM navigation payload byte budget exceeded"))?;
            match language {
                JvmLanguage::Kotlin if fact.domain_uri.as_str() == KOTLIN_FACTS_CAPABILITY => {
                    if !jvm_kotlin_fact(&fact.fact_key) {
                        return Ok(());
                    }
                    let payload: Value = read_canonical(store, &fact.payload)?;
                    project_kotlin(&fact.fact_key, &payload, &mut captured)?;
                }
                JvmLanguage::Java if fact.domain_uri.as_str() == JAVA_COMPILER_FACTS_CAPABILITY => {
                    let payload: JavaCompilerFact = read_canonical(store, &fact.payload)?;
                    project_java(&fact.fact_key, payload, &mut captured)?;
                }
                _ => {}
            }
            Ok(())
        })?;
    }
    Ok(captured)
}

fn jvm_kotlin_fact(fact_key: &str) -> bool {
    [
        "kotlin:descriptor:",
        "kotlin:descriptor-boundary:",
        "kotlin:relation:",
        "kotlin:relation-boundary:",
    ]
    .iter()
    .any(|prefix| fact_key.starts_with(prefix))
}

fn project_java(
    fact_key: &str,
    fact: JavaCompilerFact,
    captured: &mut CapturedFacts,
) -> Result<(), ClewError> {
    match fact {
        JavaCompilerFact::Declaration {
            declaration_kind,
            symbol_identity,
            modifiers,
            file,
            start,
            end,
            resolution,
            ..
        } if resolution == "COMPILER_EXACT" => {
            let callable_family = java_callable_family(&symbol_identity);
            let member_mode = if declaration_kind == "CONSTRUCTOR" {
                JvmMemberMode::Constructor
            } else if modifiers.iter().any(|modifier| modifier == "STATIC") {
                JvmMemberMode::Static
            } else if callable_family.is_some() {
                JvmMemberMode::Instance
            } else {
                JvmMemberMode::Unknown
            };
            captured.declarations.push(JvmDeclaration {
                language: JvmLanguage::Java,
                fact_key: fact_key.into(),
                raw_identity: symbol_identity.clone(),
                exact_identity: Some(symbol_identity),
                callable_family,
                member_mode,
                anchor: anchor(file, start, end)?,
                obligations: Vec::new(),
            });
        }
        JavaCompilerFact::Relation {
            relation_kind,
            source_identity,
            target_identity,
            file,
            start,
            end,
            resolution,
            ..
        } if resolution == "COMPILER_EXACT" => {
            captured.references.push(JvmReference {
                language: JvmLanguage::Java,
                fact_key: fact_key.into(),
                relation_kind,
                source_identity,
                raw_target: target_identity.clone(),
                exact_target: Some(target_identity.clone()),
                callable_family: java_callable_family(&target_identity),
                anchor: anchor(file, start, end)?,
            });
        }
        _ => {}
    }
    Ok(())
}

fn project_kotlin(
    fact_key: &str,
    payload: &Value,
    captured: &mut CapturedFacts,
) -> Result<(), ClewError> {
    match validate_kotlin_semantic_payload(payload)? {
        KotlinSemanticPayloadKind::DeclarationDescriptor => {
            let kind = required(payload, "declarationKind")?;
            let raw = required(payload, "symbolIdentity")?.to_owned();
            let callable = payload.get("compilerCallableId").and_then(Value::as_str);
            let class = payload.get("compilerClassId").and_then(Value::as_str);
            let descriptor = payload
                .get("jvmDescriptor")
                .and_then(Value::as_str)
                .or_else(|| raw.split_once("#jvm:").map(|(_, descriptor)| descriptor));
            let (exact_identity, family, mode, obligations) = match kind {
                "CLASS" => (
                    class.and_then(kotlin_class_identity),
                    None,
                    JvmMemberMode::Unknown,
                    Vec::new(),
                ),
                "FUNCTION" => (
                    callable.zip(descriptor).and_then(|(callable, descriptor)| {
                        kotlin_callable_identity(callable, descriptor)
                    }),
                    callable.and_then(kotlin_callable_family),
                    JvmMemberMode::Unknown,
                    Vec::new(),
                ),
                "CONSTRUCTOR" => (
                    callable.zip(descriptor).and_then(|(callable, descriptor)| {
                        kotlin_callable_identity(callable, descriptor)
                    }),
                    callable.and_then(kotlin_callable_family),
                    JvmMemberMode::Constructor,
                    Vec::new(),
                ),
                "PROPERTY" | "MUTABLE_PROPERTY" => (
                    None,
                    None,
                    JvmMemberMode::Unknown,
                    vec!["KOTLIN_PROPERTY_JVM_ACCESSOR_NOT_PROJECTED".into()],
                ),
                _ => return Ok(()),
            };
            captured.declarations.push(JvmDeclaration {
                language: JvmLanguage::Kotlin,
                fact_key: fact_key.into(),
                raw_identity: raw,
                exact_identity,
                callable_family: family,
                member_mode: mode,
                anchor: payload_anchor(payload)?,
                obligations,
            });
        }
        KotlinSemanticPayloadKind::DeclarationRelation => {
            let relation_kind = required(payload, "kind")?.to_owned();
            if !matches!(
                relation_kind.as_str(),
                "CALLS" | "CONSTRUCTS" | "REFERENCES" | "TYPE_USES"
            ) {
                return Ok(());
            }
            let source_identity = required(payload, "owner")?.to_owned();
            let raw_target = required(payload, "target")?.to_owned();
            captured.references.push(JvmReference {
                language: JvmLanguage::Kotlin,
                fact_key: fact_key.into(),
                relation_kind,
                source_identity,
                raw_target: raw_target.clone(),
                exact_target: None,
                callable_family: kotlin_callable_family(&raw_target),
                anchor: payload_anchor(payload)?,
            });
        }
        KotlinSemanticPayloadKind::DeclarationDescriptorBoundary
        | KotlinSemanticPayloadKind::DeclarationRelationBoundary => {
            let code = required(payload, "code")?.to_owned();
            let source_identity = payload
                .get("owner")
                .or_else(|| payload.get("symbolIdentity"))
                .and_then(Value::as_str)
                .unwrap_or("<unbound>")
                .to_owned();
            let raw_target = payload
                .get("target")
                .or_else(|| payload.get("subject"))
                .and_then(Value::as_str)
                .unwrap_or("<unbound>")
                .to_owned();
            captured.boundaries.push(JvmNavigationBoundary {
                code,
                source_language: JvmLanguage::Kotlin,
                target_language: JvmLanguage::Java,
                source_identity,
                raw_target,
                candidate_count: 0,
                required_checks: vec!["VERIFY_KOTLIN_COMPILER_BOUNDARY".into()],
            });
        }
    }
    Ok(())
}

fn semantic_input_authority(
    store: &CasStore,
    ready: &ReadyGeneration,
    language: JvmLanguage,
) -> Result<String, ClewError> {
    let manifest: DerivedAnalysisInputManifest =
        read_canonical(store, &ready.derived_input_manifest)?;
    manifest.verify(store)?;
    if manifest.provider_models.len() != 1 {
        return Err(corrupt(
            "JVM generation has ambiguous provider model authority",
        ));
    }
    let model: Value = read_canonical(store, &manifest.provider_models[0].build_model.model)?;
    let field = match language {
        JvmLanguage::Kotlin => "semanticInputManifestHash",
        JvmLanguage::Java => "modelDigest",
    };
    let digest = model
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| corrupt("JVM model omits semantic input authority"))?;
    validate_digest(digest)?;
    Ok(digest.into())
}

fn combined_semantic_authority(
    generations: &[JvmNavigationGeneration],
) -> Result<String, ClewError> {
    canonical::hash(&json!({
        "schema":"codeclew-jvm-consumer-semantic-authority/1.0",
        "generations":generations,
    }))
    .map_err(internal)
}

fn validate_generation_binding(
    ready: &ReadyGenerationSet,
    session: &SessionAuthority,
    binding: &crate::thread_context::ThreadMemberContextBinding,
) -> Result<(), ClewError> {
    if ready.runtime_key != session.runtime_key
        || ready.base_revision != session.base_revision
        || binding.base_revision != session.base_revision
        || binding.compilations != session.compilations
    {
        return Err(ClewError::new(
            ErrorCode::BindingChanged,
            "JVM navigation generation differs from session/context authority",
        ));
    }
    Ok(())
}

fn validate_request(
    thread: &ThreadAuthority,
    request: &ThreadCallablesRequest,
) -> Result<(), ClewError> {
    for value in [
        &request.task_id,
        &request.pair_id,
        &request.provider_member,
        &request.consumer_member,
    ] {
        if value.is_empty()
            || value.len() > 128
            || value
                .bytes()
                .any(|byte| !byte.is_ascii_alphanumeric() && !matches!(byte, b'-' | b'_' | b'.'))
        {
            return Err(invalid("JVM navigation identifier is invalid"));
        }
    }
    if request.provider_member == request.consumer_member
        || request.terms.is_empty()
        || request.terms.len() > 256
        || request.terms.iter().any(|term| {
            term.trim().is_empty()
                || term.len() > 4096
                || term.chars().any(char::is_control)
                || !crate::text_authority::is_nfc(term)
        })
        || !thread
            .members
            .iter()
            .any(|member| member.member_alias == request.provider_member)
        || !thread
            .members
            .iter()
            .any(|member| member.member_alias == request.consumer_member)
    {
        return Err(invalid("JVM navigation request is invalid"));
    }
    Ok(())
}

fn payload_anchor(payload: &Value) -> Result<Option<JvmSourceAnchor>, ClewError> {
    let Some(path) = payload.get("file").and_then(Value::as_str) else {
        return Ok(None);
    };
    anchor(
        path.to_owned(),
        payload.get("start").and_then(Value::as_u64),
        payload.get("end").and_then(Value::as_u64),
    )
}

fn anchor(
    path: String,
    start: Option<u64>,
    end: Option<u64>,
) -> Result<Option<JvmSourceAnchor>, ClewError> {
    let parsed = Path::new(&path);
    if parsed.is_absolute()
        || parsed
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(corrupt("JVM fact source anchor is not repository-relative"));
    }
    Ok(Some(JvmSourceAnchor { path, start, end }))
}

fn required<'a>(payload: &'a Value, field: &str) -> Result<&'a str, ClewError> {
    payload
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| corrupt("validated JVM fact omits a required string"))
}

fn read_canonical<T: for<'de> Deserialize<'de> + Serialize>(
    store: &CasStore,
    object: &CasObject,
) -> Result<T, ClewError> {
    let limit = usize::try_from(object.size)
        .map_err(|_| budget("JVM navigation CAS object exceeds host size"))?;
    let lease = store.read(object, limit)?;
    let value = serde_json::from_slice(lease.bytes())
        .map_err(|_| corrupt("JVM navigation CAS object is not JSON"))?;
    if canonical::bytes(&value).map_err(internal)? != lease.bytes() {
        return Err(corrupt("JVM navigation CAS object is not canonical"));
    }
    Ok(value)
}

fn validate_digest(value: &str) -> Result<(), ClewError> {
    if value.len() != 71
        || !value.starts_with("sha256:")
        || !value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(corrupt("JVM semantic input digest is invalid"));
    }
    Ok(())
}

fn invalid(message: &str) -> ClewError {
    ClewError::new(ErrorCode::InvalidInput, message)
}

fn corrupt(message: &str) -> ClewError {
    ClewError::new(ErrorCode::StateCorrupt, message)
}

fn budget(message: &str) -> ClewError {
    ClewError::new(ErrorCode::SliceBudgetExceeded, message)
}

fn internal(error: impl std::fmt::Display) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kotlin_routing_accepts_only_descriptor_and_relation_facts() {
        for fact_key in [
            "kotlin:descriptor:one",
            "kotlin:descriptor-boundary:one",
            "kotlin:relation:one",
            "kotlin:relation-boundary:one",
        ] {
            assert!(jvm_kotlin_fact(fact_key), "{fact_key}");
        }
        for fact_key in [
            "kotlin:file:one",
            "kotlin:metadata:one",
            "kotlin:cfg:one",
            "java:declaration:one",
        ] {
            assert!(!jvm_kotlin_fact(fact_key), "{fact_key}");
        }
    }

    #[test]
    fn java_static_and_instance_modes_remain_distinct() {
        let mut captured = CapturedFacts {
            language: JvmLanguage::Java,
            generations: Vec::new(),
            declarations: Vec::new(),
            references: Vec::new(),
            boundaries: Vec::new(),
        };
        for (name, modifiers) in [
            ("staticCall", vec!["PUBLIC", "STATIC"]),
            ("call", vec!["PUBLIC"]),
        ] {
            project_java(
                "java:declaration:test",
                JavaCompilerFact::Declaration {
                    schema: "codeclew-java-compiler-fact/1.0".into(),
                    declaration_kind: "METHOD".into(),
                    symbol_identity: format!("method:class:api.Service#{name}()V"),
                    owner_identity: "class:api.Service".into(),
                    jvm_descriptor: Some("()V".into()),
                    modifiers: modifiers.into_iter().map(str::to_owned).collect(),
                    annotations: Vec::new(),
                    interfaces: Vec::new(),
                    superclass: None,
                    file: "src/main/java/api/Service.java".into(),
                    start: Some(1),
                    end: Some(2),
                    resolution: "COMPILER_EXACT".into(),
                },
                &mut captured,
            )
            .unwrap();
        }
        assert_eq!(captured.declarations.len(), 2);
        assert_eq!(captured.declarations[0].member_mode, JvmMemberMode::Static);
        assert_eq!(
            captured.declarations[1].member_mode,
            JvmMemberMode::Instance
        );
    }
}
