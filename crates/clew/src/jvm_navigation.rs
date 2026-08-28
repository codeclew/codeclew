//! Conservative Kotlin/Java navigation over compiler facts.
//!
//! A JVM identity match is useful to an agent, but it is not artifact
//! ownership and it is not a compatibility verdict.  This module keeps those
//! axes separate: a consumer compilation can prove the target selected on its
//! exact classpath while the provider repository remains only a candidate
//! owner until a future artifact authority binds the two.

use serde::{Deserialize, Serialize};

pub const JVM_NAVIGATION_SCHEMA: &str = "codeclew-jvm-navigation/1.0";
pub const MAX_JVM_NAVIGATION_RESULTS: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum JvmLanguage {
    Kotlin,
    Java,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum JvmIdentityAuthority {
    ExactDescriptor,
    ExactUniqueCallableFamily,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum JvmMemberMode {
    Static,
    Instance,
    Constructor,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum JvmEvidenceAxis {
    Exact,
    Unverified,
    NotAssessed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JvmSourceAnchor {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JvmDeclaration {
    pub language: JvmLanguage,
    pub fact_key: String,
    pub raw_identity: String,
    pub exact_identity: Option<String>,
    pub callable_family: Option<String>,
    pub member_mode: JvmMemberMode,
    pub anchor: Option<JvmSourceAnchor>,
    pub obligations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JvmReference {
    pub language: JvmLanguage,
    pub fact_key: String,
    pub relation_kind: String,
    pub source_identity: String,
    pub raw_target: String,
    pub exact_target: Option<String>,
    pub callable_family: Option<String>,
    pub anchor: Option<JvmSourceAnchor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JvmNavigationAxes {
    pub consumer_classpath: JvmEvidenceAxis,
    pub provider_declaration: JvmEvidenceAxis,
    pub artifact_ownership: JvmEvidenceAxis,
    pub nullability_contract: JvmEvidenceAxis,
    pub compatibility: JvmEvidenceAxis,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JvmNavigationMatch {
    pub relation_kind: String,
    pub source_language: JvmLanguage,
    pub target_language: JvmLanguage,
    pub source_identity: String,
    pub target_identity: String,
    pub consumer_fact_key: String,
    pub provider_fact_key: String,
    pub identity_authority: JvmIdentityAuthority,
    pub member_mode: JvmMemberMode,
    pub consumer_classpath_authority_digest: String,
    pub consumer_anchor: Option<JvmSourceAnchor>,
    pub provider_anchor: Option<JvmSourceAnchor>,
    pub axes: JvmNavigationAxes,
    pub obligations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JvmNavigationBoundary {
    pub code: String,
    pub source_language: JvmLanguage,
    pub target_language: JvmLanguage,
    pub source_identity: String,
    pub raw_target: String,
    pub candidate_count: usize,
    pub required_checks: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JvmResolution {
    pub matches: Vec<JvmNavigationMatch>,
    pub boundaries: Vec<JvmNavigationBoundary>,
    pub truncated: bool,
}

/// Resolves only compiler-emitted references against compiler-emitted
/// declarations.  Family-only Kotlin references are exact only when the
/// selected provider contains one eligible descriptor.  An overload never
/// wins by order or by text similarity.
pub fn resolve(
    references: &[JvmReference],
    declarations: &[JvmDeclaration],
    consumer_classpath_authority_digest: &str,
    terms: &[String],
) -> JvmResolution {
    let mut matches = Vec::new();
    let mut boundaries = Vec::new();
    let normalized_terms = terms
        .iter()
        .map(|term| term.to_lowercase())
        .collect::<Vec<_>>();

    for reference in references.iter().filter(|reference| {
        normalized_terms.is_empty()
            || normalized_terms.iter().any(|term| {
                reference.source_identity.to_lowercase().contains(term)
                    || reference.raw_target.to_lowercase().contains(term)
            })
    }) {
        let (candidates, authority) = if let Some(identity) = &reference.exact_target {
            (
                declarations
                    .iter()
                    .filter(|candidate| candidate.exact_identity.as_ref() == Some(identity))
                    .collect::<Vec<_>>(),
                JvmIdentityAuthority::ExactDescriptor,
            )
        } else if let Some(family) = &reference.callable_family {
            (
                declarations
                    .iter()
                    .filter(|candidate| candidate.callable_family.as_ref() == Some(family))
                    .collect::<Vec<_>>(),
                JvmIdentityAuthority::ExactUniqueCallableFamily,
            )
        } else {
            (Vec::new(), JvmIdentityAuthority::ExactDescriptor)
        };

        if candidates.len() == 1 {
            let target = candidates[0];
            let mut obligations = target.obligations.clone();
            obligations.extend([
                "VERIFY_ARTIFACT_OWNERSHIP_BEFORE_SERVICE_CLAIM".into(),
                "VERIFY_JVM_NULLABILITY_CONTRACT".into(),
            ]);
            if target.member_mode == JvmMemberMode::Unknown {
                obligations.push("VERIFY_STATIC_INSTANCE_MAPPING".into());
            }
            obligations.sort();
            obligations.dedup();
            matches.push(JvmNavigationMatch {
                relation_kind: reference.relation_kind.clone(),
                source_language: reference.language,
                target_language: target.language,
                source_identity: reference.source_identity.clone(),
                target_identity: target
                    .exact_identity
                    .clone()
                    .unwrap_or_else(|| target.raw_identity.clone()),
                consumer_fact_key: reference.fact_key.clone(),
                provider_fact_key: target.fact_key.clone(),
                identity_authority: authority,
                member_mode: target.member_mode,
                consumer_classpath_authority_digest: consumer_classpath_authority_digest.into(),
                consumer_anchor: reference.anchor.clone(),
                provider_anchor: target.anchor.clone(),
                axes: JvmNavigationAxes {
                    consumer_classpath: JvmEvidenceAxis::Exact,
                    provider_declaration: JvmEvidenceAxis::Exact,
                    artifact_ownership: JvmEvidenceAxis::Unverified,
                    nullability_contract: JvmEvidenceAxis::Unverified,
                    compatibility: JvmEvidenceAxis::NotAssessed,
                },
                obligations,
            });
        } else {
            boundaries.push(JvmNavigationBoundary {
                code: if candidates.len() > 1 {
                    "OVERLOAD_TARGET_AMBIGUOUS".into()
                } else {
                    "TARGET_DECLARATION_NOT_FOUND".into()
                },
                source_language: reference.language,
                target_language: opposite(reference.language),
                source_identity: reference.source_identity.clone(),
                raw_target: reference.raw_target.clone(),
                candidate_count: candidates.len(),
                required_checks: if candidates.len() > 1 {
                    vec!["CAPTURE_TARGET_JVM_DESCRIPTOR".into()]
                } else {
                    vec!["VERIFY_PROVIDER_DECLARATION_AND_ARTIFACT".into()]
                },
            });
        }
    }

    matches.sort_by(|left, right| {
        (
            &left.source_identity,
            &left.target_identity,
            &left.relation_kind,
        )
            .cmp(&(
                &right.source_identity,
                &right.target_identity,
                &right.relation_kind,
            ))
    });
    matches.dedup();
    boundaries.sort_by(|left, right| {
        (&left.source_identity, &left.raw_target, &left.code).cmp(&(
            &right.source_identity,
            &right.raw_target,
            &right.code,
        ))
    });
    boundaries.dedup();
    let truncated = matches.len().saturating_add(boundaries.len()) > MAX_JVM_NAVIGATION_RESULTS;
    if truncated {
        let keep_matches = matches.len().min(MAX_JVM_NAVIGATION_RESULTS);
        matches.truncate(keep_matches);
        boundaries.truncate(MAX_JVM_NAVIGATION_RESULTS.saturating_sub(keep_matches));
    }
    JvmResolution {
        matches,
        boundaries,
        truncated,
    }
}

pub fn java_callable_family(identity: &str) -> Option<String> {
    let (prefix, descriptor) = identity.rsplit_once('(')?;
    (!prefix.is_empty() && !descriptor.is_empty()).then(|| prefix.to_owned())
}

pub fn kotlin_class_identity(class_id: &str) -> Option<String> {
    let (package, relative) = class_id.rsplit_once('/')?;
    if package.is_empty() || relative.is_empty() || relative.contains('/') {
        return None;
    }
    let package = package.replace('/', ".");
    let relative = relative.replace('.', "$");
    Some(format!("class:{package}.{relative}"))
}

pub fn kotlin_callable_identity(callable_id: &str, descriptor: &str) -> Option<String> {
    let (owner, name) = callable_id.rsplit_once('.')?;
    let descriptor = descriptor.get(descriptor.find('(')?..)?;
    if name.is_empty() {
        return None;
    }
    let owner = kotlin_class_identity(owner)?;
    Some(format!("method:{owner}#{name}{descriptor}"))
}

pub fn kotlin_callable_family(callable_id: &str) -> Option<String> {
    let (owner, name) = callable_id.rsplit_once('.')?;
    if name.is_empty() {
        return None;
    }
    let owner = kotlin_class_identity(owner)?;
    let binary_simple_name = owner.strip_prefix("class:")?.rsplit(['.', '$']).next()?;
    let jvm_name = if name == binary_simple_name {
        "<init>"
    } else {
        name
    };
    Some(format!("method:{owner}#{jvm_name}"))
}

fn opposite(language: JvmLanguage) -> JvmLanguage {
    match language {
        JvmLanguage::Kotlin => JvmLanguage::Java,
        JvmLanguage::Java => JvmLanguage::Kotlin,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anchor(path: &str) -> Option<JvmSourceAnchor> {
        Some(JvmSourceAnchor {
            path: path.into(),
            start: Some(1),
            end: Some(2),
        })
    }

    #[test]
    fn kotlin_caller_resolves_one_java_instance_method_without_ownership_promotion() {
        let declarations = vec![JvmDeclaration {
            language: JvmLanguage::Java,
            fact_key: "java:declaration:fetch".into(),
            raw_identity: "method:class:api.Service#fetch(Ljava/lang/String;)Ljava/lang/String;"
                .into(),
            exact_identity: Some(
                "method:class:api.Service#fetch(Ljava/lang/String;)Ljava/lang/String;".into(),
            ),
            callable_family: Some("method:class:api.Service#fetch".into()),
            member_mode: JvmMemberMode::Instance,
            anchor: anchor("src/main/java/api/Service.java"),
            obligations: vec![],
        }];
        let references = vec![JvmReference {
            language: JvmLanguage::Kotlin,
            fact_key: "kotlin:relation:fetch".into(),
            relation_kind: "CALLS".into(),
            source_identity: "client/Client.load".into(),
            raw_target: "api/Service.fetch".into(),
            exact_target: None,
            callable_family: kotlin_callable_family("api/Service.fetch"),
            anchor: anchor("src/main/kotlin/client/Client.kt"),
        }];
        let result = resolve(
            &references,
            &declarations,
            "sha256:classpath",
            &["fetch".into()],
        );
        assert_eq!(result.matches.len(), 1);
        assert_eq!(
            result.matches[0].identity_authority,
            JvmIdentityAuthority::ExactUniqueCallableFamily
        );
        assert_eq!(
            result.matches[0].axes.artifact_ownership,
            JvmEvidenceAxis::Unverified
        );
        assert_eq!(
            result.matches[0].axes.compatibility,
            JvmEvidenceAxis::NotAssessed
        );
    }

    #[test]
    fn java_caller_resolves_kotlin_descriptor_and_keeps_nullability_explicit() {
        let identity = kotlin_callable_identity(
            "api/KotlinService.fetch",
            "(Ljava/lang/String;)Ljava/lang/String;",
        )
        .unwrap();
        let result = resolve(
            &[JvmReference {
                language: JvmLanguage::Java,
                fact_key: "java:relation:fetch".into(),
                relation_kind: "CALLS".into(),
                source_identity: "method:class:client.Client#load()V".into(),
                raw_target: identity.clone(),
                exact_target: Some(identity.clone()),
                callable_family: java_callable_family(&identity),
                anchor: anchor("src/main/java/client/Client.java"),
            }],
            &[JvmDeclaration {
                language: JvmLanguage::Kotlin,
                fact_key: "kotlin:declaration:fetch".into(),
                raw_identity:
                    "callable:api/KotlinService.fetch#jvm:(Ljava/lang/String;)Ljava/lang/String;"
                        .into(),
                exact_identity: Some(identity.clone()),
                callable_family: java_callable_family(&identity),
                member_mode: JvmMemberMode::Unknown,
                anchor: anchor("src/main/kotlin/api/KotlinService.kt"),
                obligations: vec![],
            }],
            "sha256:java-model",
            &["KotlinService".into()],
        );
        assert_eq!(result.matches.len(), 1);
        assert!(
            result.matches[0]
                .obligations
                .contains(&"VERIFY_JVM_NULLABILITY_CONTRACT".into())
        );
        assert!(
            result.matches[0]
                .obligations
                .contains(&"VERIFY_STATIC_INSTANCE_MAPPING".into())
        );
    }

    #[test]
    fn family_only_reference_never_guesses_between_overloads() {
        let declarations = ["I", "Ljava/lang/String;"]
            .into_iter()
            .map(|parameter| JvmDeclaration {
                language: JvmLanguage::Java,
                fact_key: format!("java:declaration:{parameter}"),
                raw_identity: format!("method:class:api.Service#read({parameter})I"),
                exact_identity: Some(format!("method:class:api.Service#read({parameter})I")),
                callable_family: Some("method:class:api.Service#read".into()),
                member_mode: JvmMemberMode::Instance,
                anchor: anchor("src/main/java/api/Service.java"),
                obligations: vec![],
            })
            .collect::<Vec<_>>();
        let result = resolve(
            &[JvmReference {
                language: JvmLanguage::Kotlin,
                fact_key: "kotlin:relation:read".into(),
                relation_kind: "CALLS".into(),
                source_identity: "client/Client.read".into(),
                raw_target: "api/Service.read".into(),
                exact_target: None,
                callable_family: kotlin_callable_family("api/Service.read"),
                anchor: None,
            }],
            &declarations,
            "sha256:classpath",
            &["read".into()],
        );
        assert!(result.matches.is_empty());
        assert_eq!(result.boundaries[0].code, "OVERLOAD_TARGET_AMBIGUOUS");
        assert_eq!(result.boundaries[0].candidate_count, 2);
    }

    #[test]
    fn missing_generated_or_local_declaration_stays_a_boundary() {
        let result = resolve(
            &[JvmReference {
                language: JvmLanguage::Java,
                fact_key: "java:relation:generated".into(),
                relation_kind: "CALLS".into(),
                source_identity: "method:class:client.Client#load()V".into(),
                raw_target: "method:class:generated.Mapper#map()V".into(),
                exact_target: Some("method:class:generated.Mapper#map()V".into()),
                callable_family: Some("method:class:generated.Mapper#map".into()),
                anchor: anchor("src/main/java/client/Client.java"),
            }],
            &[],
            "sha256:classpath",
            &["Mapper".into()],
        );
        assert!(result.matches.is_empty());
        assert_eq!(result.boundaries.len(), 1);
        assert_eq!(result.boundaries[0].code, "TARGET_DECLARATION_NOT_FOUND");
        assert_eq!(result.boundaries[0].candidate_count, 0);
        assert_eq!(
            result.boundaries[0].required_checks,
            ["VERIFY_PROVIDER_DECLARATION_AND_ARTIFACT"]
        );
    }

    #[test]
    fn constructors_and_nested_classes_have_deterministic_binary_identities() {
        assert_eq!(
            kotlin_callable_identity("api/Outer.Inner.<init>", "(I)V").as_deref(),
            Some("method:class:api.Outer$Inner#<init>(I)V")
        );
        assert_eq!(
            kotlin_class_identity("api/Outer.Inner").as_deref(),
            Some("class:api.Outer$Inner")
        );
        assert!(kotlin_callable_identity("api/topLevel", "()V").is_none());
        assert_eq!(
            kotlin_callable_identity("api/Service.read", "read(I)I").as_deref(),
            Some("method:class:api.Service#read(I)I")
        );
        assert_eq!(
            kotlin_callable_family("api/Outer.Inner.Inner").as_deref(),
            Some("method:class:api.Outer$Inner#<init>")
        );
    }
}
