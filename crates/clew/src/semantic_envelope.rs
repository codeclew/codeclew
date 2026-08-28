//! Language-neutral facts without a language-neutrality claim.
//!
//! The envelope exposes only six concepts useful to an agent across service
//! boundaries. Language-specific bytes stay behind an immutable CAS reference;
//! composition can reduce authority but can never manufacture certainty or a
//! relationship.

use crate::canonical;
use crate::cas::{CasObject, CasStore};
use crate::error::{ClewError, ErrorCode};
use crate::thread_callables::{
    BoundaryFact, CallableCompilationBinding, CallableFact, CallableFactProvenance,
    DeclarationFact, DeclarationKind, GraphCoverage, RelationshipAuthority, SourceAnchor, UseFact,
};
use serde::{Deserialize, Serialize};

pub const SEMANTIC_FACT_SCHEMA: &str = "codeclew-semantic-fact/1.0";
pub const SEMANTIC_ENVELOPE_SCHEMA: &str = "codeclew-semantic-envelope/1.0";
pub const SEMANTIC_QUERY_RESULT_SCHEMA: &str = "codeclew-semantic-query-result/1.0";
pub const KOTLIN_CALLABLE_PAYLOAD_SCHEMA: &str = "codeclew-kotlin-callable-fact/1.0";
pub const MAX_ENVELOPE_FACTS: usize = 131_072;
pub const MAX_QUERY_TERMS: usize = 64;
pub const MAX_QUERY_RESULTS: usize = 256;
pub const MAX_OPAQUE_PAYLOAD_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SemanticCertainty {
    Exact,
    Declared,
    Unsure,
}

impl SemanticCertainty {
    /// Conservative composition: the least authoritative input wins.
    pub fn meet(self, other: Self) -> Self {
        use SemanticCertainty::{Declared, Exact, Unsure};
        match (self, other) {
            (Unsure, _) | (_, Unsure) => Unsure,
            (Declared, _) | (_, Declared) => Declared,
            (Exact, Exact) => Exact,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SemanticCompleteness {
    CompleteSupportedSubset,
    Partial,
    Unknown,
}

impl SemanticCompleteness {
    /// Conservative composition: unknown and partial inputs remain visible.
    pub fn meet(self, other: Self) -> Self {
        use SemanticCompleteness::{CompleteSupportedSubset, Partial, Unknown};
        match (self, other) {
            (Unknown, _) | (_, Unknown) => Unknown,
            (Partial, _) | (_, Partial) => Partial,
            (CompleteSupportedSubset, CompleteSupportedSubset) => CompleteSupportedSubset,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExtractorAuthority {
    pub extractor_id: String,
    pub adapter_digest: String,
    pub runtime_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SemanticFactAuthority {
    pub member_alias: String,
    pub repository_namespace: String,
    pub revision: String,
    pub language: String,
    pub extractor: ExtractorAuthority,
    pub compilation_id: String,
    pub generation_id: String,
    pub generation_ref: CasObject,
    pub input_fact_key: String,
    pub input_payload_ref: CasObject,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_ref: Option<CasObject>,
    pub certainty: SemanticCertainty,
    pub completeness: SemanticCompleteness,
    pub obligations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OpaqueLanguagePayload {
    pub language: String,
    pub payload_schema: String,
    pub payload_version: String,
    pub payload_ref: CasObject,
}

impl OpaqueLanguagePayload {
    pub fn read_exact(&self, store: &CasStore) -> Result<Vec<u8>, ClewError> {
        Ok(store
            .read(&self.payload_ref, MAX_OPAQUE_PAYLOAD_BYTES)?
            .bytes()
            .to_vec())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "SCREAMING_SNAKE_CASE",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum SemanticFactContent {
    Symbol {
        identity: String,
        owner_identity: String,
        declaration_kind: String,
    },
    DeclarationShape {
        symbol_identity: String,
        owner_identity: String,
        declaration_kind: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        shape_digest: Option<String>,
    },
    Relation {
        relation_kind: String,
        source_identity: String,
        target_identity: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        target_repository_namespace: Option<String>,
        authority: SemanticCertainty,
    },
    SourceAnchor {
        path: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        start: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        end: Option<u64>,
        content_ref: CasObject,
    },
    Boundary {
        stage: String,
        code: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        subject: Option<String>,
    },
    ChangeObservation {
        observation_identity: String,
        code: String,
        before_symbols: Vec<String>,
        after_symbols: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SemanticFact {
    pub schema: String,
    pub fact_id: String,
    pub authority: SemanticFactAuthority,
    pub content: SemanticFactContent,
    pub opaque_payload: OpaqueLanguagePayload,
}

impl SemanticFact {
    pub fn new(
        authority: SemanticFactAuthority,
        content: SemanticFactContent,
        opaque_payload: OpaqueLanguagePayload,
    ) -> Result<Self, ClewError> {
        if authority.language != opaque_payload.language {
            return Err(invalid(
                "semantic fact language differs from its opaque payload authority",
            ));
        }
        if opaque_payload.payload_schema != opaque_payload.payload_ref.object_schema {
            return Err(invalid(
                "semantic fact payload schema differs from its CAS authority",
            ));
        }
        let fact_id = fact_identity(&authority, &content, &opaque_payload)?;
        Ok(Self {
            schema: SEMANTIC_FACT_SCHEMA.into(),
            fact_id,
            authority,
            content,
            opaque_payload,
        })
    }

    fn validate(&self) -> Result<(), ClewError> {
        if self.schema != SEMANTIC_FACT_SCHEMA
            || self.authority.language != self.opaque_payload.language
            || self.opaque_payload.payload_schema != self.opaque_payload.payload_ref.object_schema
            || self.fact_id != fact_identity(&self.authority, &self.content, &self.opaque_payload)?
        {
            return Err(invalid("semantic fact authority is not canonical"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SemanticEnvelope {
    pub schema: String,
    pub envelope_id: String,
    pub facts: Vec<SemanticFact>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticQuery {
    pub terms: Vec<String>,
    pub max_results: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SemanticQueryResult {
    pub schema: String,
    pub envelope_id: String,
    pub terms: Vec<String>,
    pub facts: Vec<SemanticFact>,
    pub truncated: bool,
}

impl SemanticEnvelope {
    pub fn new(mut facts: Vec<SemanticFact>) -> Result<Self, ClewError> {
        if facts.len() > MAX_ENVELOPE_FACTS {
            return Err(resource_limit("semantic envelope fact budget exceeded"));
        }
        for fact in &facts {
            fact.validate()?;
        }
        facts.sort_by(|left, right| left.fact_id.cmp(&right.fact_id));
        for pair in facts.windows(2) {
            if pair[0].fact_id == pair[1].fact_id && pair[0] != pair[1] {
                return Err(invalid("semantic fact id has conflicting content"));
            }
        }
        facts.dedup();
        let envelope_id = canonical::hash(&("codeclew-semantic-envelope-identity/1.0", &facts))
            .map_err(internal)?;
        Ok(Self {
            schema: SEMANTIC_ENVELOPE_SCHEMA.into(),
            envelope_id,
            facts,
        })
    }

    /// Query only common fields. Opaque language bytes are never parsed or
    /// searched, so a future adapter can add fields without changing results.
    pub fn query(&self, request: SemanticQuery) -> Result<SemanticQueryResult, ClewError> {
        self.validate()?;
        if request.terms.is_empty() || request.terms.len() > MAX_QUERY_TERMS {
            return Err(invalid(
                "semantic query term count is outside its bounded profile",
            ));
        }
        if request.max_results == 0 || request.max_results > MAX_QUERY_RESULTS {
            return Err(invalid(
                "semantic query result limit is outside its bounded profile",
            ));
        }
        let mut terms = request
            .terms
            .into_iter()
            .map(|term| term.trim().to_lowercase())
            .collect::<Vec<_>>();
        if terms.iter().any(|term| term.is_empty() || term.len() > 256) {
            return Err(invalid("semantic query contains an invalid term"));
        }
        terms.sort();
        terms.dedup();

        let mut matches = self
            .facts
            .iter()
            .filter(|fact| {
                let fields = searchable_fields(fact);
                terms
                    .iter()
                    .all(|term| fields.iter().any(|field| field.contains(term)))
            })
            .cloned()
            .collect::<Vec<_>>();
        let truncated = matches.len() > request.max_results;
        matches.truncate(request.max_results);
        Ok(SemanticQueryResult {
            schema: SEMANTIC_QUERY_RESULT_SCHEMA.into(),
            envelope_id: self.envelope_id.clone(),
            terms,
            facts: matches,
            truncated,
        })
    }

    fn validate(&self) -> Result<(), ClewError> {
        if self.schema != SEMANTIC_ENVELOPE_SCHEMA || self.facts.len() > MAX_ENVELOPE_FACTS {
            return Err(invalid(
                "semantic envelope is outside its canonical profile",
            ));
        }
        for pair in self.facts.windows(2) {
            if pair[0].fact_id >= pair[1].fact_id {
                return Err(invalid("semantic envelope facts are not canonical"));
            }
        }
        for fact in &self.facts {
            fact.validate()?;
        }
        let expected = canonical::hash(&("codeclew-semantic-envelope-identity/1.0", &self.facts))
            .map_err(internal)?;
        if self.envelope_id != expected {
            return Err(invalid("semantic envelope identity is stale"));
        }
        Ok(())
    }
}

/// Losslessly project a validated Kotlin callable row. The complete callable
/// row is retained as opaque CAS bytes; common fields are deliberately small.
pub fn project_kotlin_callable(
    store: &CasStore,
    fact: &CallableFact,
    compilation: &CallableCompilationBinding,
) -> Result<Vec<SemanticFact>, ClewError> {
    let (provenance, certainty, completeness, mut obligations) = match fact {
        CallableFact::Declaration(row) => {
            let exact = row.exact_eligible
                && compilation.descriptor_coverage == GraphCoverage::CompleteSupportedSubset;
            (
                &row.provenance,
                if exact {
                    SemanticCertainty::Exact
                } else {
                    SemanticCertainty::Unsure
                },
                completeness(compilation.descriptor_coverage),
                row.uncertainty_reasons.clone(),
            )
        }
        CallableFact::Use(row) => {
            let certainty = if !row.exact_eligible {
                SemanticCertainty::Unsure
            } else {
                match row.relationship_authority {
                    RelationshipAuthority::VerifiedSameSnapshotCompilationDependency
                        if compilation.relation_coverage
                            == GraphCoverage::CompleteSupportedSubset =>
                    {
                        SemanticCertainty::Exact
                    }
                    RelationshipAuthority::DeclaredTopology => SemanticCertainty::Declared,
                    RelationshipAuthority::VerifiedSameSnapshotCompilationDependency
                    | RelationshipAuthority::Unbound => SemanticCertainty::Unsure,
                }
            };
            (
                &row.provenance,
                certainty,
                completeness(compilation.relation_coverage),
                row.uncertainty_reasons.clone(),
            )
        }
        CallableFact::Boundary(row) => (
            &row.provenance,
            SemanticCertainty::Unsure,
            SemanticCompleteness::Partial,
            row.required_checks.clone(),
        ),
    };
    validate_binding(provenance, compilation)?;
    if obligations.is_empty() {
        match certainty {
            SemanticCertainty::Unsure => {
                obligations.push("VERIFY_LANGUAGE_SPECIFIC_AUTHORITY".into())
            }
            SemanticCertainty::Declared => obligations.push("VERIFY_DECLARED_RELATIONSHIP".into()),
            SemanticCertainty::Exact => {}
        }
    }
    obligations.sort();
    obligations.dedup();

    let payload_bytes = canonical::bytes(fact).map_err(internal)?;
    let payload_ref = store.put(KOTLIN_CALLABLE_PAYLOAD_SCHEMA, &payload_bytes)?;
    let opaque_payload = OpaqueLanguagePayload {
        language: "language:kotlin".into(),
        payload_schema: KOTLIN_CALLABLE_PAYLOAD_SCHEMA.into(),
        payload_version: "1.0".into(),
        payload_ref,
    };
    let authority = authority(
        provenance,
        compilation,
        certainty,
        completeness,
        obligations,
    );

    let mut content = match fact {
        CallableFact::Declaration(row) => declaration_content(row),
        CallableFact::Use(row) => use_content(row, certainty),
        CallableFact::Boundary(row) => boundary_content(row),
    };
    if let Some(source) = &provenance.source {
        content.push(source_content(source)?);
    }
    content
        .into_iter()
        .map(|content| SemanticFact::new(authority.clone(), content, opaque_payload.clone()))
        .collect()
}

fn authority(
    provenance: &CallableFactProvenance,
    compilation: &CallableCompilationBinding,
    certainty: SemanticCertainty,
    completeness: SemanticCompleteness,
    obligations: Vec<String>,
) -> SemanticFactAuthority {
    SemanticFactAuthority {
        member_alias: provenance.member_alias.clone(),
        repository_namespace: provenance.repository_namespace.clone(),
        revision: provenance.base_revision.clone(),
        language: "language:kotlin".into(),
        extractor: ExtractorAuthority {
            extractor_id: compilation.extractor_id.clone(),
            adapter_digest: compilation.adapter_digest.clone(),
            runtime_digest: compilation.runtime_digest.clone(),
        },
        compilation_id: compilation.compilation_id.clone(),
        generation_id: compilation.generation_id.clone(),
        generation_ref: compilation.generation_ref.clone(),
        input_fact_key: provenance.input_fact_key.clone(),
        input_payload_ref: provenance.input_payload_ref.clone(),
        source_ref: provenance
            .source
            .as_ref()
            .map(|source| source.content_ref.clone()),
        certainty,
        completeness,
        obligations,
    }
}

fn declaration_content(row: &DeclarationFact) -> Vec<SemanticFactContent> {
    let declaration_kind = declaration_kind(row.declaration_kind).to_owned();
    vec![
        SemanticFactContent::Symbol {
            identity: row.symbol_identity.clone(),
            owner_identity: row.owner_identity.clone(),
            declaration_kind: declaration_kind.clone(),
        },
        SemanticFactContent::DeclarationShape {
            symbol_identity: row.symbol_identity.clone(),
            owner_identity: row.owner_identity.clone(),
            declaration_kind,
            shape_digest: row.shape_digest.clone(),
        },
    ]
}

fn use_content(row: &UseFact, certainty: SemanticCertainty) -> Vec<SemanticFactContent> {
    vec![SemanticFactContent::Relation {
        relation_kind: row.relation_kind.clone(),
        source_identity: row.source_owner.clone(),
        target_identity: row
            .target_symbol_identity
            .clone()
            .unwrap_or_else(|| row.target_callable_id.clone()),
        target_repository_namespace: row.target_repository_namespace.clone(),
        authority: certainty,
    }]
}

fn boundary_content(row: &BoundaryFact) -> Vec<SemanticFactContent> {
    vec![SemanticFactContent::Boundary {
        stage: row.stage.clone(),
        code: row.code.clone(),
        subject: row.subject.clone(),
    }]
}

fn source_content(source: &SourceAnchor) -> Result<SemanticFactContent, ClewError> {
    let path = std::path::Path::new(&source.path);
    if path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(invalid("semantic source anchor is not repository-relative"));
    }
    Ok(SemanticFactContent::SourceAnchor {
        path: source.path.clone(),
        start: source.start,
        end: source.end,
        content_ref: source.content_ref.clone(),
    })
}

fn fact_identity(
    authority: &SemanticFactAuthority,
    content: &SemanticFactContent,
    opaque_payload: &OpaqueLanguagePayload,
) -> Result<String, ClewError> {
    canonical::hash(&(
        "codeclew-semantic-fact-identity/1.0",
        &authority,
        &content,
        &opaque_payload,
    ))
    .map_err(internal)
}

fn validate_binding(
    provenance: &CallableFactProvenance,
    compilation: &CallableCompilationBinding,
) -> Result<(), ClewError> {
    if provenance.compilation_id != compilation.compilation_id
        || provenance.generation_id != compilation.generation_id
        || provenance.generation_ref != compilation.generation_ref
    {
        return Err(invalid(
            "Kotlin callable provenance does not match compilation authority",
        ));
    }
    Ok(())
}

fn completeness(coverage: GraphCoverage) -> SemanticCompleteness {
    match coverage {
        GraphCoverage::CompleteSupportedSubset => SemanticCompleteness::CompleteSupportedSubset,
        GraphCoverage::Partial => SemanticCompleteness::Partial,
    }
}

fn declaration_kind(kind: DeclarationKind) -> &'static str {
    match kind {
        DeclarationKind::Function => "FUNCTION",
        DeclarationKind::Constructor => "CONSTRUCTOR",
        DeclarationKind::Class => "CLASS",
        DeclarationKind::Property => "PROPERTY",
        DeclarationKind::MutableProperty => "MUTABLE_PROPERTY",
    }
}

fn searchable_fields(fact: &SemanticFact) -> Vec<String> {
    let mut fields = vec![
        fact.authority.language.to_lowercase(),
        fact.authority.repository_namespace.to_lowercase(),
        fact.authority.member_alias.to_lowercase(),
    ];
    match &fact.content {
        SemanticFactContent::Symbol {
            identity,
            owner_identity,
            declaration_kind,
        }
        | SemanticFactContent::DeclarationShape {
            symbol_identity: identity,
            owner_identity,
            declaration_kind,
            ..
        } => {
            fields.extend(
                [identity, owner_identity, declaration_kind].map(|value| value.to_lowercase()),
            );
        }
        SemanticFactContent::Relation {
            relation_kind,
            source_identity,
            target_identity,
            target_repository_namespace,
            ..
        } => {
            fields.extend(
                [relation_kind, source_identity, target_identity].map(|value| value.to_lowercase()),
            );
            if let Some(namespace) = target_repository_namespace {
                fields.push(namespace.to_lowercase());
            }
        }
        SemanticFactContent::SourceAnchor { path, .. } => fields.push(path.to_lowercase()),
        SemanticFactContent::Boundary {
            stage,
            code,
            subject,
        } => {
            fields.extend([stage, code].map(|value| value.to_lowercase()));
            if let Some(subject) = subject {
                fields.push(subject.to_lowercase());
            }
        }
        SemanticFactContent::ChangeObservation {
            observation_identity,
            code,
            before_symbols,
            after_symbols,
        } => {
            fields.extend([observation_identity, code].map(|value| value.to_lowercase()));
            fields.extend(before_symbols.iter().map(|value| value.to_lowercase()));
            fields.extend(after_symbols.iter().map(|value| value.to_lowercase()));
        }
    }
    fields
}

fn invalid(message: &str) -> ClewError {
    ClewError::new(ErrorCode::InvalidInput, message)
}

fn resource_limit(message: &str) -> ClewError {
    ClewError::new(ErrorCode::ResourceLimit, message)
}

fn internal(error: impl std::fmt::Display) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::StateAuthority;
    use serde_json::json;

    fn object(schema: &str, seed: &str) -> CasObject {
        CasObject::for_bytes(schema, seed.as_bytes()).unwrap()
    }

    fn compilation(
        descriptor: GraphCoverage,
        relation: GraphCoverage,
    ) -> CallableCompilationBinding {
        CallableCompilationBinding {
            compilation_id: ":/main".into(),
            generation_id: "generation:one".into(),
            generation_ref: object("generation/1", "generation"),
            semantic_authority: "COMPILER_BACKED".into(),
            extractor_id: "kotlin-worker".into(),
            adapter_digest: "sha256:adapter".into(),
            runtime_digest: "sha256:runtime".into(),
            descriptor_coverage: descriptor,
            relation_coverage: relation,
        }
    }

    fn provenance(binding: &CallableCompilationBinding) -> CallableFactProvenance {
        CallableFactProvenance {
            member_alias: "service".into(),
            repository_namespace: "repo:service".into(),
            session_id: "session:one".into(),
            session_authority_digest: "sha256:session".into(),
            base_revision: "0123456789012345678901234567890123456789".into(),
            compilation_id: binding.compilation_id.clone(),
            generation_id: binding.generation_id.clone(),
            generation_ref: binding.generation_ref.clone(),
            input_fact_key: "kotlin:one".into(),
            input_payload_ref: object("kotlin-semantic/3", "payload"),
            source: Some(SourceAnchor {
                path: "src/main/kotlin/example/Service.kt".into(),
                start: Some(10),
                end: Some(20),
                content_ref: object("source/1", "source"),
            }),
        }
    }

    fn declaration(binding: &CallableCompilationBinding, name: &str) -> CallableFact {
        CallableFact::Declaration(DeclarationFact {
            schema: "codeclew-kotlin-callable-declaration/1.0".into(),
            fact_id: format!("fact:{name}"),
            provenance: provenance(binding),
            declaration_kind: DeclarationKind::Function,
            symbol_identity: format!("example.Service.{name}()"),
            compiler_callable_id: Some(format!("example.Service.{name}")),
            compiler_class_id: None,
            jvm_descriptor: Some("()V".into()),
            owner_identity: "example.Service".into(),
            containment: vec!["example.Service".into()],
            projected_shape: json!({"futureKotlinField":{"nested":true}}),
            shape_digest: Some(format!("sha256:{name}")),
            exact_eligible: true,
            uncertainty_reasons: vec![],
        })
    }

    fn store() -> (tempfile::TempDir, CasStore) {
        let root = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(root.path().join("v2")).unwrap();
        let store = CasStore::open(&state).unwrap();
        (root, store)
    }

    #[test]
    fn kotlin_declaration_projects_common_concepts_and_roundtrips_losslessly() {
        let (_root, store) = store();
        let binding = compilation(
            GraphCoverage::CompleteSupportedSubset,
            GraphCoverage::CompleteSupportedSubset,
        );
        let source = declaration(&binding, "load");
        let expected = canonical::bytes(&source).unwrap();
        let projected = project_kotlin_callable(&store, &source, &binding).unwrap();

        assert!(matches!(
            projected[0].authority.certainty,
            SemanticCertainty::Exact
        ));
        assert_eq!(projected.len(), 3);
        assert!(
            projected
                .iter()
                .any(|fact| matches!(fact.content, SemanticFactContent::Symbol { .. }))
        );
        assert!(
            projected
                .iter()
                .any(|fact| matches!(fact.content, SemanticFactContent::DeclarationShape { .. }))
        );
        assert!(
            projected
                .iter()
                .any(|fact| matches!(fact.content, SemanticFactContent::SourceAnchor { .. }))
        );
        assert_eq!(
            projected[0].opaque_payload.read_exact(&store).unwrap(),
            expected
        );
    }

    #[test]
    fn opaque_future_language_bytes_roundtrip_without_interpretation() {
        let (_root, store) = store();
        let bytes = br#"{"schema":"future/9","unknown":{"nested":[3,2,1]}}"#;
        let payload = OpaqueLanguagePayload {
            language: "language:future".into(),
            payload_schema: "future-language-fact/9.0".into(),
            payload_version: "9.0".into(),
            payload_ref: store.put("future-language-fact/9.0", bytes).unwrap(),
        };
        assert_eq!(payload.read_exact(&store).unwrap(), bytes);
    }

    #[test]
    fn composition_never_promotes_certainty_or_completeness() {
        assert_eq!(
            SemanticCertainty::Exact.meet(SemanticCertainty::Declared),
            SemanticCertainty::Declared
        );
        assert_eq!(
            SemanticCertainty::Exact.meet(SemanticCertainty::Unsure),
            SemanticCertainty::Unsure
        );
        assert_eq!(
            SemanticCompleteness::CompleteSupportedSubset.meet(SemanticCompleteness::Partial),
            SemanticCompleteness::Partial
        );
        assert_eq!(
            SemanticCompleteness::CompleteSupportedSubset.meet(SemanticCompleteness::Unknown),
            SemanticCompleteness::Unknown
        );
    }

    #[test]
    fn projector_does_not_invent_relations() {
        let (_root, store) = store();
        let binding = compilation(
            GraphCoverage::CompleteSupportedSubset,
            GraphCoverage::CompleteSupportedSubset,
        );
        let projected =
            project_kotlin_callable(&store, &declaration(&binding, "load"), &binding).unwrap();
        assert!(
            !projected
                .iter()
                .any(|fact| matches!(fact.content, SemanticFactContent::Relation { .. }))
        );
    }

    #[test]
    fn declared_and_unbound_relations_keep_their_weaker_authority() {
        let (_root, store) = store();
        let binding = compilation(
            GraphCoverage::CompleteSupportedSubset,
            GraphCoverage::CompleteSupportedSubset,
        );
        for (relationship_authority, expected) in [
            (
                RelationshipAuthority::DeclaredTopology,
                SemanticCertainty::Declared,
            ),
            (RelationshipAuthority::Unbound, SemanticCertainty::Unsure),
        ] {
            let row = CallableFact::Use(UseFact {
                schema: "codeclew-kotlin-callable-use/1.0".into(),
                fact_id: "use:one".into(),
                provenance: provenance(&binding),
                relation_kind: "CALLS".into(),
                source_owner: "example.Source".into(),
                target_callable_id: "example.Target.call".into(),
                target_symbol_identity: None,
                target_repository_namespace: None,
                target_resolution: crate::thread_callables::TargetResolution::CallableFamily,
                relationship_authority,
                relation_evidence: json!({}),
                exact_eligible: true,
                uncertainty_reasons: vec![],
            });
            let projected = project_kotlin_callable(&store, &row, &binding).unwrap();
            assert_eq!(projected[0].authority.certainty, expected);
            assert!(!projected[0].authority.obligations.is_empty());
        }
    }

    #[test]
    fn query_is_deterministic_bounded_and_ignores_opaque_bytes() {
        let (_root, store) = store();
        let binding = compilation(
            GraphCoverage::CompleteSupportedSubset,
            GraphCoverage::CompleteSupportedSubset,
        );
        let mut facts =
            project_kotlin_callable(&store, &declaration(&binding, "load"), &binding).unwrap();
        facts.extend(
            project_kotlin_callable(&store, &declaration(&binding, "save"), &binding).unwrap(),
        );
        let forward = SemanticEnvelope::new(facts.clone()).unwrap();
        facts.reverse();
        let reverse = SemanticEnvelope::new(facts).unwrap();
        assert_eq!(forward, reverse);

        let result = forward
            .query(SemanticQuery {
                terms: vec!["service".into()],
                max_results: 2,
            })
            .unwrap();
        assert_eq!(result.facts.len(), 2);
        assert!(result.truncated);
        assert!(
            forward
                .query(SemanticQuery {
                    terms: vec!["futurekotlinfield".into()],
                    max_results: 10,
                })
                .unwrap()
                .facts
                .is_empty()
        );
    }
}
