use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::canonical::{CanonicalError, canonical_json_bytes, evidence_merkle_root, sha256_digest};
use crate::protocol::*;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationError {
    pub path: String,
    pub code: &'static str,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[error("evidence validation failed with {} error(s)", .0.len())]
pub struct ValidationErrors(pub Vec<ValidationError>);

impl ValidationErrors {
    pub fn errors(&self) -> &[ValidationError] {
        &self.0
    }
}

pub trait Validate {
    fn validate(&self) -> Result<(), ValidationErrors>;
}

/// A transport envelope used by adapters and conformance tests. It introduces
/// no language semantics; every semantic statement remains an EvidenceFact.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceBundle {
    pub snapshot: WorkspaceAnalysisSnapshot,
    pub capabilities: Vec<CapabilityDecision>,
    pub batches: Vec<EvidenceBatch>,
    pub obligation_graphs: Vec<ObligationGraph>,
    pub verification_receipts: Vec<VerificationReceipt>,
    pub impact_receipts: Vec<ImpactReceipt>,
}

pub fn validate_bundle(bundle: &EvidenceBundle) -> Result<(), ValidationErrors> {
    bundle.validate()
}

/// Types with a `content_digest` are hashed with that field cleared. Nested
/// digests remain part of their parent's identity.
pub trait ContentAddressed: Clone + Serialize {
    fn content_digest(&self) -> &str;
    fn set_content_digest(&mut self, digest: String);

    fn computed_content_digest(&self) -> Result<String, CanonicalError> {
        let mut unsigned = self.clone();
        unsigned.set_content_digest(String::new());
        Ok(sha256_digest(canonical_json_bytes(&unsigned)?))
    }
}

pub fn seal_content_digest<T: ContentAddressed>(value: &mut T) -> Result<String, CanonicalError> {
    let digest = value.computed_content_digest()?;
    value.set_content_digest(digest.clone());
    Ok(digest)
}

macro_rules! content_addressed {
    ($($type:ty),+ $(,)?) => {$ (
        impl ContentAddressed for $type {
            fn content_digest(&self) -> &str { &self.content_digest }
            fn set_content_digest(&mut self, digest: String) { self.content_digest = digest; }
        }
    )+ };
}

content_addressed!(
    Assumption,
    Occurrence,
    Scope,
    Boundary,
    Coverage,
    Refusal,
    CapabilityDescriptor,
    CapabilityDecision,
    EvidenceFact,
    EvidenceBatch,
    Obligation,
    ObligationGraph,
    ClaimSpec,
    VerificationReceipt,
    ImpactReceipt,
);

impl WorkspaceAnalysisSnapshot {
    /// Compute snapshot identity over all analysis inputs. Creation time is
    /// audit metadata and deliberately excluded; ordered compiler flags remain.
    pub fn computed_snapshot_id(&self) -> Result<String, CanonicalError> {
        let mut identity = self.clone();
        identity.snapshot_id.clear();
        identity.metadata = None;
        Ok(sha256_digest(canonical_json_bytes(&identity)?))
    }

    pub fn seal_snapshot_id(&mut self) -> Result<String, CanonicalError> {
        let digest = self.computed_snapshot_id()?;
        self.snapshot_id = digest.clone();
        Ok(digest)
    }
}

#[derive(Default)]
struct Errors(Vec<ValidationError>);

impl Errors {
    fn push(&mut self, path: impl Into<String>, code: &'static str, message: impl Into<String>) {
        self.0.push(ValidationError {
            path: path.into(),
            code,
            message: message.into(),
        });
    }

    fn required(&mut self, present: bool, path: &str) {
        if !present {
            self.push(path, "required", "field is required");
        }
    }

    fn finish(self) -> Result<(), ValidationErrors> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err(ValidationErrors(self.0))
        }
    }

    fn append(&mut self, prefix: &str, result: Result<(), ValidationErrors>) {
        if let Err(errors) = result {
            self.0.extend(errors.0.into_iter().map(|mut error| {
                error.path = if error.path.is_empty() {
                    prefix.to_owned()
                } else {
                    format!("{prefix}.{}", error.path)
                };
                error
            }));
        }
    }
}

fn require_text(errors: &mut Errors, value: &str, path: &str) {
    if value.trim().is_empty() {
        errors.push(path, "required", "non-empty text is required");
    }
}

fn require_uri(errors: &mut Errors, value: &str, path: &str) {
    require_text(errors, value, path);
    // Contracts may use an RFC URI (`urn:...`, `https://...`) or the frozen
    // Codeclew namespaced/versioned form (`codeclew.relation/calls/1`). What
    // matters to the core is exact opaque identity, not URI interpretation.
    if !value.contains(':')
        && (!value.contains('/') || value.bytes().any(|byte| byte.is_ascii_whitespace()))
    {
        errors.push(
            path,
            "invalid_uri",
            "identifier must be an RFC URI or a namespaced/versioned path",
        );
    }
}

fn is_digest(value: &str) -> bool {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return false;
    };
    hex.len() == 64
        && hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn require_digest(errors: &mut Errors, value: &str, path: &str) {
    if !is_digest(value) {
        errors.push(
            path,
            "invalid_digest",
            "expected sha256:<64 lowercase hex digits>",
        );
    }
}

fn require_content_digest<T: ContentAddressed>(errors: &mut Errors, value: &T, path: &str) {
    require_digest(errors, value.content_digest(), path);
    match value.computed_content_digest() {
        Ok(computed) if computed != value.content_digest() => errors.push(
            path,
            "digest_mismatch",
            format!(
                "declared {} but computed {computed}",
                value.content_digest()
            ),
        ),
        Err(error) => errors.push(path, "canonicalization_failed", error.to_string()),
        _ => {}
    }
}

fn require_sorted_unique<T, K: Ord + fmt::Debug>(
    errors: &mut Errors,
    values: &[T],
    path: &str,
    key: impl Fn(&T) -> K,
) {
    if values.windows(2).any(|pair| key(&pair[0]) >= key(&pair[1])) {
        errors.push(
            path,
            "not_canonical_set",
            "set-like repeated field must be strictly sorted and duplicate-free",
        );
    }
}

fn validate_schema(errors: &mut Errors, value: Option<&SchemaRef>, path: &str) {
    let Some(schema) = value else {
        errors.required(false, path);
        return;
    };
    require_uri(errors, &schema.uri, &format!("{path}.uri"));
    if schema.major == 0 {
        errors.push(
            format!("{path}.major"),
            "invalid_version",
            "major version must be non-zero",
        );
    }
    require_digest(
        errors,
        &schema.specification_digest,
        &format!("{path}.specificationDigest"),
    );
}

fn validate_payload(errors: &mut Errors, value: &TypedPayload, path: &str) {
    validate_schema(errors, value.schema.as_ref(), &format!("{path}.schema"));
    require_text(errors, &value.media_type, &format!("{path}.mediaType"));
    require_digest(
        errors,
        &value.content_digest,
        &format!("{path}.contentDigest"),
    );
    let computed = sha256_digest(&value.canonical_bytes);
    if value.content_digest != computed {
        errors.push(
            format!("{path}.contentDigest"),
            "digest_mismatch",
            format!("payload bytes compute to {computed}"),
        );
    }
    if value.media_type == "application/json" {
        match serde_json::from_slice::<serde_json::Value>(&value.canonical_bytes) {
            Ok(parsed) => match canonical_json_bytes(&parsed) {
                Ok(canonical) if canonical != value.canonical_bytes => errors.push(
                    format!("{path}.canonicalBytes"),
                    "non_canonical_json",
                    "JSON payload must use canonical key ordering and whitespace",
                ),
                Err(error) => errors.push(
                    format!("{path}.canonicalBytes"),
                    "canonicalization_failed",
                    error.to_string(),
                ),
                _ => {}
            },
            Err(error) => errors.push(
                format!("{path}.canonicalBytes"),
                "invalid_json",
                error.to_string(),
            ),
        }
    }
}

fn validate_evidence_ref(errors: &mut Errors, value: &EvidenceRef, path: &str) {
    require_uri(errors, &value.kind_uri, &format!("{path}.kindUri"));
    require_digest(
        errors,
        &value.content_digest,
        &format!("{path}.contentDigest"),
    );
}

fn validate_assumption(errors: &mut Errors, value: &Assumption, path: &str) {
    require_uri(errors, &value.kind_uri, &format!("{path}.kindUri"));
    if let Some(details) = value.details.as_ref() {
        validate_payload(errors, details, &format!("{path}.details"));
    } else {
        errors.required(false, &format!("{path}.details"));
    }
    require_content_digest(errors, value, &format!("{path}.contentDigest"));
}

fn validate_adapter(errors: &mut Errors, value: Option<&AdapterIdentity>, path: &str) {
    let Some(adapter) = value else {
        errors.required(false, path);
        return;
    };
    // Adapter IDs are opaque registry keys. Version and binary digest provide
    // the exact identity; forcing an RFC URI would add no semantic safety.
    require_text(errors, &adapter.adapter_id, &format!("{path}.adapterId"));
    require_text(errors, &adapter.version, &format!("{path}.version"));
    require_digest(
        errors,
        &adapter.binary_digest,
        &format!("{path}.binaryDigest"),
    );
}

fn validate_operation(errors: &mut Errors, value: Option<&OperationRef>, path: &str) {
    let Some(operation) = value else {
        errors.required(false, path);
        return;
    };
    require_uri(errors, &operation.uri, &format!("{path}.uri"));
    require_text(errors, &operation.version, &format!("{path}.version"));
    require_digest(
        errors,
        &operation.specification_digest,
        &format!("{path}.specificationDigest"),
    );
}

fn validate_range(errors: &mut Errors, value: &SourceRange, path: &str) {
    require_text(errors, &value.artifact_id, &format!("{path}.artifactId"));
    require_digest(
        errors,
        &value.artifact_content_digest,
        &format!("{path}.artifactContentDigest"),
    );
    if value.end_byte < value.start_byte {
        errors.push(
            format!("{path}.endByte"),
            "invalid_range",
            "endByte precedes startByte",
        );
    }
}

fn validate_entity(errors: &mut Errors, value: &EntityRef, path: &str) {
    require_uri(
        errors,
        &value.adapter_namespace,
        &format!("{path}.adapterNamespace"),
    );
    require_text(errors, &value.opaque_id, &format!("{path}.opaqueId"));
    if EntityResolution::try_from(value.resolution).is_err()
        || value.resolution == EntityResolution::Unspecified as i32
    {
        errors.push(
            format!("{path}.resolution"),
            "invalid_enum",
            "specified entity resolution required",
        );
    }
    if CoarseEntityKind::try_from(value.coarse_kind).is_err()
        || value.coarse_kind == CoarseEntityKind::Unspecified as i32
    {
        errors.push(
            format!("{path}.coarseKind"),
            "invalid_enum",
            "specified coarse entity kind required",
        );
    }
    if let Some(range) = value.primary_definition.as_ref() {
        validate_range(errors, range, &format!("{path}.primaryDefinition"));
    }
    if let Some(payload) = value.language_payload.as_ref() {
        validate_payload(errors, payload, &format!("{path}.languagePayload"));
    }
}

fn validate_occurrence(errors: &mut Errors, value: &Occurrence, path: &str) {
    require_text(
        errors,
        &value.occurrence_id,
        &format!("{path}.occurrenceId"),
    );
    if let Some(range) = value.range.as_ref() {
        validate_range(errors, range, &format!("{path}.range"));
    } else {
        errors.required(false, &format!("{path}.range"));
    }
    require_sorted_unique(errors, &value.roles, &format!("{path}.roles"), |role| *role);
    if value.roles.is_empty()
        || value.roles.iter().any(|role| {
            OccurrenceRole::try_from(*role).is_err() || *role == OccurrenceRole::Unspecified as i32
        })
    {
        errors.push(
            format!("{path}.roles"),
            "invalid_enum",
            "one or more specified roles required",
        );
    }
    if OccurrenceOrigin::try_from(value.origin).is_err()
        || value.origin == OccurrenceOrigin::Unspecified as i32
    {
        errors.push(
            format!("{path}.origin"),
            "invalid_enum",
            "specified occurrence origin required",
        );
    }
    if let Some(entity) = value.entity.as_ref() {
        validate_entity(errors, entity, &format!("{path}.entity"));
    } else {
        errors.required(false, &format!("{path}.entity"));
    }
    require_sorted_unique(
        errors,
        &value.evidence,
        &format!("{path}.evidence"),
        |item| (item.kind_uri.clone(), item.content_digest.clone()),
    );
    for (index, evidence) in value.evidence.iter().enumerate() {
        validate_evidence_ref(errors, evidence, &format!("{path}.evidence[{index}]"));
    }
    require_content_digest(errors, value, &format!("{path}.contentDigest"));
}

fn validate_scope(errors: &mut Errors, value: &Scope, path: &str) {
    require_uri(errors, &value.scope_uri, &format!("{path}.scopeUri"));
    if let Some(selector) = value.selector.as_ref() {
        validate_payload(errors, selector, &format!("{path}.selector"));
    } else {
        errors.required(false, &format!("{path}.selector"));
    }
    require_content_digest(errors, value, &format!("{path}.contentDigest"));
}

fn validate_boundary(errors: &mut Errors, value: &Boundary, path: &str) {
    require_text(errors, &value.boundary_id, &format!("{path}.boundaryId"));
    require_uri(errors, &value.kind_uri, &format!("{path}.kindUri"));
    require_text(errors, &value.origin, &format!("{path}.origin"));
    if BoundaryConsequence::try_from(value.consequence).is_err()
        || value.consequence == BoundaryConsequence::Unspecified as i32
    {
        errors.push(
            format!("{path}.consequence"),
            "invalid_enum",
            "specified boundary consequence required",
        );
    }
    if let Some(details) = value.details.as_ref() {
        validate_payload(errors, details, &format!("{path}.details"));
    }
    require_sorted_unique(
        errors,
        &value.evidence,
        &format!("{path}.evidence"),
        |item| (item.kind_uri.clone(), item.content_digest.clone()),
    );
    for (index, evidence) in value.evidence.iter().enumerate() {
        validate_evidence_ref(errors, evidence, &format!("{path}.evidence[{index}]"));
    }
    require_content_digest(errors, value, &format!("{path}.contentDigest"));
}

fn validate_coverage(errors: &mut Errors, value: &Coverage, path: &str) {
    let enumeration = Enumeration::try_from(value.enumeration).ok();
    let approximation = Approximation::try_from(value.approximation).ok();
    if enumeration.is_none() || enumeration == Some(Enumeration::Unspecified) {
        errors.push(
            format!("{path}.enumeration"),
            "invalid_enum",
            "specified enumeration required",
        );
    }
    if approximation.is_none() || approximation == Some(Approximation::Unspecified) {
        errors.push(
            format!("{path}.approximation"),
            "invalid_enum",
            "specified approximation required",
        );
    }
    require_sorted_unique(errors, &value.scopes, &format!("{path}.scopes"), |scope| {
        (scope.scope_uri.clone(), scope.content_digest.clone())
    });
    for (index, scope) in value.scopes.iter().enumerate() {
        validate_scope(errors, scope, &format!("{path}.scopes[{index}]"));
    }
    require_sorted_unique(
        errors,
        &value.boundaries,
        &format!("{path}.boundaries"),
        |boundary| boundary.boundary_id.clone(),
    );
    for (index, boundary) in value.boundaries.iter().enumerate() {
        validate_boundary(errors, boundary, &format!("{path}.boundaries[{index}]"));
    }
    require_sorted_unique(
        errors,
        &value.assumptions,
        &format!("{path}.assumptions"),
        |assumption| assumption.content_digest.clone(),
    );
    for (index, assumption) in value.assumptions.iter().enumerate() {
        validate_assumption(errors, assumption, &format!("{path}.assumptions[{index}]"));
    }
    let incomplete_boundary = value.boundaries.iter().any(|boundary| {
        matches!(
            BoundaryConsequence::try_from(boundary.consequence),
            Ok(BoundaryConsequence::EnumerationIncomplete | BoundaryConsequence::ProofInvalid)
        )
    });
    if enumeration == Some(Enumeration::CompleteInScope) && incomplete_boundary {
        errors.push(
            format!("{path}.enumeration"),
            "false_completeness",
            "COMPLETE_IN_SCOPE conflicts with an incomplete or proof-invalid boundary",
        );
    }
    if matches!(
        enumeration,
        Some(Enumeration::Partial | Enumeration::Unknown)
    ) && value.boundaries.is_empty()
    {
        errors.push(
            format!("{path}.boundaries"),
            "missing_boundary",
            "PARTIAL or UNKNOWN coverage must explain at least one explicit boundary",
        );
    }
    if enumeration == Some(Enumeration::CompleteInScope)
        && approximation == Some(Approximation::Heuristic)
    {
        errors.push(
            format!("{path}.approximation"),
            "false_completeness",
            "heuristic evidence cannot claim complete enumeration",
        );
    }
    require_content_digest(errors, value, &format!("{path}.contentDigest"));
}

impl Validate for WorkspaceAnalysisSnapshot {
    fn validate(&self) -> Result<(), ValidationErrors> {
        let mut errors = Errors::default();
        validate_schema(&mut errors, self.schema.as_ref(), "schema");
        require_digest(&mut errors, &self.snapshot_id, "snapshotId");
        require_digest(
            &mut errors,
            &self.repository_tree_digest,
            "repositoryTreeDigest",
        );
        require_uri(&mut errors, &self.build_system_uri, "buildSystemUri");
        require_digest(&mut errors, &self.build_model_digest, "buildModelDigest");
        require_digest(
            &mut errors,
            &self.build_configuration_digest,
            "buildConfigurationDigest",
        );
        require_digest(
            &mut errors,
            &self.dependency_graph_digest,
            "dependencyGraphDigest",
        );
        require_digest(
            &mut errors,
            &self.generated_sources_manifest_digest,
            "generatedSourcesManifestDigest",
        );
        validate_adapter(&mut errors, self.adapter.as_ref(), "adapter");
        if self.sources.is_empty() {
            errors.push(
                "sources",
                "required",
                "snapshot must contain its analyzed source manifest",
            );
        }
        require_sorted_unique(&mut errors, &self.sources, "sources", |source| {
            source.artifact_id.clone()
        });
        for (index, source) in self.sources.iter().enumerate() {
            let path = format!("sources[{index}]");
            require_text(
                &mut errors,
                &source.artifact_id,
                &format!("{path}.artifactId"),
            );
            require_text(
                &mut errors,
                &source.normalized_path,
                &format!("{path}.normalizedPath"),
            );
            if source.normalized_path.starts_with('/')
                || source.normalized_path.split('/').any(|part| part == "..")
                || source.normalized_path.contains('\\')
            {
                errors.push(
                    format!("{path}.normalizedPath"),
                    "unsafe_path",
                    "source path must be normalized, relative, and slash-separated",
                );
            }
            require_digest(
                &mut errors,
                &source.content_digest,
                &format!("{path}.contentDigest"),
            );
            if ArtifactOrigin::try_from(source.origin).is_err()
                || source.origin == ArtifactOrigin::Unspecified as i32
            {
                errors.push(
                    format!("{path}.origin"),
                    "invalid_enum",
                    "specified artifact origin required",
                );
            }
            require_sorted_unique(
                &mut errors,
                &source.generated_from,
                &format!("{path}.generatedFrom"),
                |map| (map.source_artifact_id.clone(), map.map_digest.clone()),
            );
            for (map_index, map) in source.generated_from.iter().enumerate() {
                let map_path = format!("{path}.generatedFrom[{map_index}]");
                require_text(
                    &mut errors,
                    &map.source_artifact_id,
                    &format!("{map_path}.sourceArtifactId"),
                );
                require_digest(
                    &mut errors,
                    &map.map_digest,
                    &format!("{map_path}.mapDigest"),
                );
                if let Some(mapping) = map.mapping.as_ref() {
                    validate_payload(&mut errors, mapping, &format!("{map_path}.mapping"));
                } else {
                    errors.required(false, &format!("{map_path}.mapping"));
                }
            }
        }
        if let Some(toolchain) = self.toolchain.as_ref() {
            require_uri(&mut errors, &toolchain.tool_uri, "toolchain.toolUri");
            require_text(&mut errors, &toolchain.version, "toolchain.version");
            require_digest(
                &mut errors,
                &toolchain.distribution_digest,
                "toolchain.distributionDigest",
            );
            require_sorted_unique(
                &mut errors,
                &toolchain.plugins,
                "toolchain.plugins",
                |plugin| plugin.key.clone(),
            );
            if let Some(payload) = toolchain.language_payload.as_ref() {
                validate_payload(&mut errors, payload, "toolchain.languagePayload");
            }
            if let Some(payload) = toolchain.language_payload.as_ref() {
                validate_payload(&mut errors, payload, "toolchain.languagePayload");
            }
        } else {
            errors.required(false, "toolchain");
        }
        require_sorted_unique(&mut errors, &self.targets, "targets", |target| {
            target.target_id.clone()
        });
        for (index, target) in self.targets.iter().enumerate() {
            let path = format!("targets[{index}]");
            require_text(&mut errors, &target.target_id, &format!("{path}.targetId"));
            require_digest(
                &mut errors,
                &target.configuration_digest,
                &format!("{path}.configurationDigest"),
            );
            require_sorted_unique(
                &mut errors,
                &target.enabled_features,
                &format!("{path}.enabledFeatures"),
                Clone::clone,
            );
            // target.compiler_flags is intentionally not treated as a set.
            if let Some(payload) = target.language_payload.as_ref() {
                validate_payload(&mut errors, payload, &format!("{path}.languagePayload"));
            }
            if let Some(payload) = target.language_payload.as_ref() {
                validate_payload(&mut errors, payload, &format!("{path}.languagePayload"));
            }
        }
        require_sorted_unique(
            &mut errors,
            &self.relevant_environment,
            "relevantEnvironment",
            |entry| entry.key.clone(),
        );
        if let Some(metadata) = self.metadata.as_ref() {
            require_text(&mut errors, &metadata.created_at, "metadata.createdAt");
        }
        match self.computed_snapshot_id() {
            Ok(computed) if computed != self.snapshot_id => errors.push(
                "snapshotId",
                "digest_mismatch",
                format!("snapshot analysis inputs compute to {computed}"),
            ),
            Err(error) => errors.push("snapshotId", "canonicalization_failed", error.to_string()),
            _ => {}
        }
        errors.finish()
    }
}

impl Validate for Coverage {
    fn validate(&self) -> Result<(), ValidationErrors> {
        let mut errors = Errors::default();
        validate_coverage(&mut errors, self, "");
        errors.finish()
    }
}

impl Validate for CapabilityDescriptor {
    fn validate(&self) -> Result<(), ValidationErrors> {
        let mut errors = Errors::default();
        validate_schema(&mut errors, self.schema.as_ref(), "schema");
        let Some(key) = self.key.as_ref() else {
            errors.required(false, "key");
            return errors.finish();
        };
        require_text(&mut errors, &key.language_id, "key.languageId");
        validate_adapter(&mut errors, key.adapter.as_ref(), "key.adapter");
        require_digest(&mut errors, &key.snapshot_id, "key.snapshotId");
        require_digest(&mut errors, &key.toolchain_digest, "key.toolchainDigest");
        require_digest(
            &mut errors,
            &key.build_configuration_digest,
            "key.buildConfigurationDigest",
        );
        require_digest(&mut errors, &key.target_digest, "key.targetDigest");
        validate_operation(&mut errors, key.operation.as_ref(), "key.operation");
        if EvidenceGrade::try_from(key.grade).is_err()
            || matches!(
                EvidenceGrade::try_from(key.grade),
                Ok(EvidenceGrade::Unspecified | EvidenceGrade::Unknown)
            )
        {
            errors.push(
                "key.grade",
                "invalid_grade",
                "supported capability needs an explicit evidence grade",
            );
        }
        require_sorted_unique(
            &mut errors,
            &self.input_domain_uris,
            "inputDomainUris",
            Clone::clone,
        );
        for (index, uri) in self.input_domain_uris.iter().enumerate() {
            require_uri(&mut errors, uri, &format!("inputDomainUris[{index}]"));
        }
        validate_schema(&mut errors, self.output_schema.as_ref(), "outputSchema");
        if let Some(coverage) = self.guaranteed_coverage.as_ref() {
            validate_coverage(&mut errors, coverage, "guaranteedCoverage");
        } else {
            errors.required(false, "guaranteedCoverage");
        }
        require_sorted_unique(
            &mut errors,
            &self.required_capability_digests,
            "requiredCapabilityDigests",
            Clone::clone,
        );
        for (index, digest) in self.required_capability_digests.iter().enumerate() {
            require_digest(
                &mut errors,
                digest,
                &format!("requiredCapabilityDigests[{index}]"),
            );
        }
        require_sorted_unique(
            &mut errors,
            &self.known_boundary_kind_uris,
            "knownBoundaryKindUris",
            Clone::clone,
        );
        for (index, assumption) in self.assumptions.iter().enumerate() {
            validate_assumption(&mut errors, assumption, &format!("assumptions[{index}]"));
        }
        require_sorted_unique(&mut errors, &self.assumptions, "assumptions", |item| {
            item.content_digest.clone()
        });
        for (field, contour) in [
            ("supportedContour", &self.supported_contour),
            ("unsupportedContour", &self.unsupported_contour),
        ] {
            require_sorted_unique(&mut errors, contour, field, |item| {
                (item.scope_uri.clone(), item.content_digest.clone())
            });
            for (index, scope) in contour.iter().enumerate() {
                validate_scope(&mut errors, scope, &format!("{field}[{index}]"));
            }
        }
        require_uri(&mut errors, &self.cost_class_uri, "costClassUri");
        require_content_digest(&mut errors, self, "contentDigest");
        errors.finish()
    }
}

impl Validate for CapabilityDecision {
    fn validate(&self) -> Result<(), ValidationErrors> {
        let mut errors = Errors::default();
        match SupportStatus::try_from(self.status) {
            Ok(SupportStatus::Supported) => {
                errors.required(self.descriptor.is_some(), "descriptor");
                if let Some(descriptor) = self.descriptor.as_ref() {
                    errors.append("descriptor", descriptor.validate());
                }
                if self.refusal.is_some() {
                    errors.push(
                        "refusal",
                        "contradictory_decision",
                        "SUPPORTED cannot carry a refusal",
                    );
                }
            }
            Ok(SupportStatus::Unsupported | SupportStatus::Unknown) => {
                errors.required(self.refusal.is_some(), "refusal");
                if let Some(refusal) = self.refusal.as_ref() {
                    validate_schema(&mut errors, refusal.schema.as_ref(), "refusal.schema");
                    require_uri(&mut errors, &refusal.reason_uri, "refusal.reasonUri");
                    require_digest(
                        &mut errors,
                        &refusal.operation_digest,
                        "refusal.operationDigest",
                    );
                    require_digest(&mut errors, &refusal.snapshot_id, "refusal.snapshotId");
                    for (index, boundary) in refusal.boundaries.iter().enumerate() {
                        validate_boundary(
                            &mut errors,
                            boundary,
                            &format!("refusal.boundaries[{index}]"),
                        );
                    }
                    require_content_digest(&mut errors, refusal, "refusal.contentDigest");
                }
            }
            _ => errors.push(
                "status",
                "invalid_enum",
                "specified support status required",
            ),
        }
        require_content_digest(&mut errors, self, "contentDigest");
        errors.finish()
    }
}

fn validate_assertion(errors: &mut Errors, value: &RelationAssertion, path: &str) {
    validate_operation(errors, value.relation.as_ref(), &format!("{path}.relation"));
    require_sorted_unique(
        errors,
        &value.operands,
        &format!("{path}.operands"),
        |operand| operand.name.clone(),
    );
    for (index, operand) in value.operands.iter().enumerate() {
        let operand_path = format!("{path}.operands[{index}]");
        require_text(errors, &operand.name, &format!("{operand_path}.name"));
        let Some(operand_value) = operand.value.as_ref() else {
            errors.required(false, &format!("{operand_path}.value"));
            continue;
        };
        match operand_value {
            operand::Value::Entity(entity) => {
                validate_entity(errors, entity, &format!("{operand_path}.entity"))
            }
            operand::Value::Occurrence(occurrence) => {
                validate_occurrence(errors, occurrence, &format!("{operand_path}.occurrence"))
            }
            operand::Value::Opaque(payload) => {
                validate_payload(errors, payload, &format!("{operand_path}.opaque"))
            }
            operand::Value::Identity(identity) => {
                require_text(errors, identity, &format!("{operand_path}.identity"))
            }
            operand::Value::Integer(_) | operand::Value::Boolean(_) => {}
        }
    }
    if Truth::try_from(value.truth).is_err() || value.truth == Truth::Unspecified as i32 {
        errors.push(
            format!("{path}.truth"),
            "invalid_enum",
            "specified truth value required (UNKNOWN is explicit)",
        );
    }
    if let Some(payload) = value.language_payload.as_ref() {
        validate_payload(errors, payload, &format!("{path}.languagePayload"));
    }
}

impl Validate for EvidenceFact {
    fn validate(&self) -> Result<(), ValidationErrors> {
        let mut errors = Errors::default();
        validate_schema(&mut errors, self.schema.as_ref(), "schema");
        require_text(&mut errors, &self.fact_id, "factId");
        require_digest(&mut errors, &self.snapshot_id, "snapshotId");
        require_digest(
            &mut errors,
            &self.capability_descriptor_digest,
            "capabilityDescriptorDigest",
        );
        if let Some(assertion) = self.assertion.as_ref() {
            validate_assertion(&mut errors, assertion, "assertion");
        } else {
            errors.required(false, "assertion");
        }
        if self.provenance.is_empty() {
            errors.push(
                "provenance",
                "required",
                "evidence fact requires exact provenance",
            );
        }
        require_sorted_unique(&mut errors, &self.provenance, "provenance", |item| {
            (item.kind_uri.clone(), item.content_digest.clone())
        });
        for (index, provenance) in self.provenance.iter().enumerate() {
            validate_evidence_ref(&mut errors, provenance, &format!("provenance[{index}]"));
        }
        if let Some(coverage) = self.coverage.as_ref() {
            validate_coverage(&mut errors, coverage, "coverage");
        } else {
            errors.required(false, "coverage");
        }
        require_content_digest(&mut errors, self, "contentDigest");
        errors.finish()
    }
}

impl Validate for EvidenceBatch {
    fn validate(&self) -> Result<(), ValidationErrors> {
        let mut errors = Errors::default();
        validate_schema(&mut errors, self.schema.as_ref(), "schema");
        require_digest(&mut errors, &self.snapshot_id, "snapshotId");
        require_digest(
            &mut errors,
            &self.capability_descriptor_digest,
            "capabilityDescriptorDigest",
        );
        require_sorted_unique(&mut errors, &self.entities, "entities", |entity| {
            (entity.adapter_namespace.clone(), entity.opaque_id.clone())
        });
        for (index, entity) in self.entities.iter().enumerate() {
            validate_entity(&mut errors, entity, &format!("entities[{index}]"));
        }
        require_sorted_unique(&mut errors, &self.occurrences, "occurrences", |item| {
            item.occurrence_id.clone()
        });
        for (index, occurrence) in self.occurrences.iter().enumerate() {
            validate_occurrence(&mut errors, occurrence, &format!("occurrences[{index}]"));
        }
        require_sorted_unique(&mut errors, &self.facts, "facts", |fact| {
            fact.fact_id.clone()
        });
        for (index, fact) in self.facts.iter().enumerate() {
            errors.append(&format!("facts[{index}]"), fact.validate());
            if fact.snapshot_id != self.snapshot_id {
                errors.push(
                    format!("facts[{index}].snapshotId"),
                    "snapshot_mismatch",
                    "fact is not bound to its batch snapshot",
                );
            }
            if fact.capability_descriptor_digest != self.capability_descriptor_digest {
                errors.push(
                    format!("facts[{index}].capabilityDescriptorDigest"),
                    "capability_mismatch",
                    "fact is not bound to its batch capability",
                );
            }
        }
        require_sorted_unique(&mut errors, &self.artifacts, "artifacts", |item| {
            (item.kind_uri.clone(), item.content_digest.clone())
        });
        for (index, artifact) in self.artifacts.iter().enumerate() {
            validate_evidence_ref(&mut errors, artifact, &format!("artifacts[{index}]"));
        }
        require_content_digest(&mut errors, self, "contentDigest");
        errors.finish()
    }
}

impl Validate for ObligationGraph {
    fn validate(&self) -> Result<(), ValidationErrors> {
        let mut errors = Errors::default();
        validate_schema(&mut errors, self.schema.as_ref(), "schema");
        require_digest(&mut errors, &self.snapshot_id, "snapshotId");
        require_digest(&mut errors, &self.intent_digest, "intentDigest");
        require_digest(
            &mut errors,
            &self.closure_capability_digest,
            "closureCapabilityDigest",
        );
        require_digest(
            &mut errors,
            &self.closure_specification_digest,
            "closureSpecificationDigest",
        );
        if self.obligations.is_empty() {
            errors.push(
                "obligations",
                "missing_closure",
                "obligation closure must contain at least one explicit obligation",
            );
        }
        if !self
            .obligations
            .iter()
            .any(|obligation| obligation.mandatory)
        {
            errors.push(
                "obligations",
                "missing_mandatory_closure",
                "obligation closure must contain at least one mandatory obligation",
            );
        }
        require_sorted_unique(&mut errors, &self.obligations, "obligations", |item| {
            item.obligation_id.clone()
        });
        let ids = self
            .obligations
            .iter()
            .map(|item| item.obligation_id.as_str())
            .collect::<BTreeSet<_>>();
        let mut dependencies = BTreeMap::<&str, Vec<&str>>::new();
        for (index, obligation) in self.obligations.iter().enumerate() {
            let path = format!("obligations[{index}]");
            require_text(
                &mut errors,
                &obligation.obligation_id,
                &format!("{path}.obligationId"),
            );
            require_digest(
                &mut errors,
                &obligation.origin_intent_digest,
                &format!("{path}.originIntentDigest"),
            );
            if obligation.origin_intent_digest != self.intent_digest {
                errors.push(
                    format!("{path}.originIntentDigest"),
                    "intent_mismatch",
                    "obligation must bind to graph intent",
                );
            }
            validate_operation(
                &mut errors,
                obligation.required_operation.as_ref(),
                &format!("{path}.requiredOperation"),
            );
            if let Some(scope) = obligation.scope.as_ref() {
                validate_scope(&mut errors, scope, &format!("{path}.scope"));
            } else {
                errors.required(false, &format!("{path}.scope"));
            }
            if let Some(precondition) = obligation.precondition.as_ref() {
                validate_payload(&mut errors, precondition, &format!("{path}.precondition"));
            }
            if let Some(postcondition) = obligation.postcondition.as_ref() {
                validate_payload(&mut errors, postcondition, &format!("{path}.postcondition"));
            }
            require_sorted_unique(
                &mut errors,
                &obligation.accepted_grades,
                &format!("{path}.acceptedGrades"),
                |grade| *grade,
            );
            for grade in &obligation.accepted_grades {
                if EvidenceGrade::try_from(*grade).is_err()
                    || matches!(
                        EvidenceGrade::try_from(*grade),
                        Ok(EvidenceGrade::Unspecified | EvidenceGrade::Unknown)
                    )
                {
                    errors.push(
                        format!("{path}.acceptedGrades"),
                        "invalid_grade",
                        "accepted grades are exact categories",
                    );
                }
            }
            require_sorted_unique(
                &mut errors,
                &obligation.dependency_ids,
                &format!("{path}.dependencyIds"),
                Clone::clone,
            );
            for dependency in &obligation.dependency_ids {
                if !ids.contains(dependency.as_str()) {
                    errors.push(
                        format!("{path}.dependencyIds"),
                        "unknown_dependency",
                        format!("unknown obligation {dependency}"),
                    );
                }
                if dependency == &obligation.obligation_id {
                    errors.push(
                        format!("{path}.dependencyIds"),
                        "cycle",
                        "obligation cannot depend on itself",
                    );
                }
            }
            dependencies.insert(
                &obligation.obligation_id,
                obligation
                    .dependency_ids
                    .iter()
                    .map(String::as_str)
                    .collect(),
            );
            let status = ObligationStatus::try_from(obligation.status).ok();
            if status.is_none() || status == Some(ObligationStatus::Unspecified) {
                errors.push(
                    format!("{path}.status"),
                    "invalid_enum",
                    "specified obligation status required",
                );
            }
            if status == Some(ObligationStatus::Satisfied)
                && obligation.evidence_fact_ids.is_empty()
            {
                errors.push(
                    format!("{path}.evidenceFactIds"),
                    "unsupported_satisfaction",
                    "SATISFIED obligation requires evidence fact ids",
                );
            }
            if matches!(
                status,
                Some(ObligationStatus::Unknown | ObligationStatus::Unsupported)
            ) && obligation.unknown_reason.trim().is_empty()
            {
                errors.push(
                    format!("{path}.unknownReason"),
                    "required",
                    "UNKNOWN or UNSUPPORTED obligation must state a reason",
                );
            }
            require_sorted_unique(
                &mut errors,
                &obligation.evidence_fact_ids,
                &format!("{path}.evidenceFactIds"),
                Clone::clone,
            );
            require_content_digest(&mut errors, obligation, &format!("{path}.contentDigest"));
        }
        let mut permanent = BTreeSet::new();
        let mut temporary = BTreeSet::new();
        for id in dependencies.keys().copied() {
            if graph_cycle(id, &dependencies, &mut permanent, &mut temporary) {
                errors.push(
                    "obligations",
                    "cycle",
                    "obligation dependency graph must be acyclic",
                );
                break;
            }
        }
        require_content_digest(&mut errors, self, "contentDigest");
        errors.finish()
    }
}

fn graph_cycle<'a>(
    id: &'a str,
    graph: &BTreeMap<&'a str, Vec<&'a str>>,
    permanent: &mut BTreeSet<&'a str>,
    temporary: &mut BTreeSet<&'a str>,
) -> bool {
    if permanent.contains(id) {
        return false;
    }
    if !temporary.insert(id) {
        return true;
    }
    if graph
        .get(id)
        .into_iter()
        .flatten()
        .any(|dependency| graph_cycle(dependency, graph, permanent, temporary))
    {
        return true;
    }
    temporary.remove(id);
    permanent.insert(id);
    false
}

fn validate_claim(errors: &mut Errors, value: &ClaimSpec, path: &str) {
    if let Some(assertion) = value.claim.as_ref() {
        validate_assertion(errors, assertion, &format!("{path}.claim"));
    } else {
        errors.required(false, &format!("{path}.claim"));
    }
    require_sorted_unique(
        errors,
        &value.accepted_grades,
        &format!("{path}.acceptedGrades"),
        |grade| *grade,
    );
    if value.accepted_grades.is_empty() {
        errors.push(
            format!("{path}.acceptedGrades"),
            "required",
            "claim must name accepted evidence grade categories",
        );
    }
    for grade in &value.accepted_grades {
        if EvidenceGrade::try_from(*grade).is_err()
            || matches!(
                EvidenceGrade::try_from(*grade),
                Ok(EvidenceGrade::Unspecified | EvidenceGrade::Unknown)
            )
        {
            errors.push(
                format!("{path}.acceptedGrades"),
                "invalid_grade",
                "accepted grades are exact categories",
            );
        }
    }
    if Enumeration::try_from(value.required_enumeration).is_err()
        || value.required_enumeration == Enumeration::Unspecified as i32
    {
        errors.push(
            format!("{path}.requiredEnumeration"),
            "invalid_enum",
            "specified enumeration required",
        );
    }
    require_sorted_unique(
        errors,
        &value.accepted_approximations,
        &format!("{path}.acceptedApproximations"),
        |approximation| *approximation,
    );
    if value.accepted_approximations.is_empty() {
        errors.push(
            format!("{path}.acceptedApproximations"),
            "required",
            "claim must name accepted approximation categories",
        );
    }
    require_uri(
        errors,
        &value.composition_rule_uri,
        &format!("{path}.compositionRuleUri"),
    );
    require_content_digest(errors, value, &format!("{path}.contentDigest"));
}

impl Validate for ClaimSpec {
    fn validate(&self) -> Result<(), ValidationErrors> {
        let mut errors = Errors::default();
        validate_claim(&mut errors, self, "");
        errors.finish()
    }
}

impl Validate for VerificationReceipt {
    fn validate(&self) -> Result<(), ValidationErrors> {
        let mut errors = Errors::default();
        validate_schema(&mut errors, self.schema.as_ref(), "schema");
        require_text(&mut errors, &self.receipt_id, "receiptId");
        require_digest(&mut errors, &self.before_snapshot_id, "beforeSnapshotId");
        if let Some(after) = self.after_snapshot_id.as_ref() {
            require_digest(&mut errors, after, "afterSnapshotId");
        }
        validate_adapter(&mut errors, self.verifier.as_ref(), "verifier");
        if let Some(claim) = self.claim.as_ref() {
            validate_claim(&mut errors, claim, "claim");
        } else {
            errors.required(false, "claim");
        }
        if ClaimResult::try_from(self.result).is_err()
            || self.result == ClaimResult::Unspecified as i32
        {
            errors.push("result", "invalid_enum", "specified claim result required");
        }
        require_digest(
            &mut errors,
            &self.obligation_graph_digest,
            "obligationGraphDigest",
        );
        if let Some(coverage) = self.coverage.as_ref() {
            validate_coverage(&mut errors, coverage, "coverage");
            if self.result == ClaimResult::Satisfied as i32
                && coverage.boundaries.iter().any(|boundary| {
                    boundary.consequence == BoundaryConsequence::ProofInvalid as i32
                })
            {
                errors.push(
                    "result",
                    "false_proof",
                    "SATISFIED receipt cannot cross a PROOF_INVALID boundary",
                );
            }
            if self.result == ClaimResult::Satisfied as i32
                && self
                    .claim
                    .as_ref()
                    .is_some_and(|claim| !receipt_coverage_meets_claim(claim, coverage))
            {
                errors.push(
                    "result",
                    "false_proof",
                    "SATISFIED receipt coverage does not meet its ClaimSpec",
                );
            }
        } else {
            errors.required(false, "coverage");
        }
        for (index, assumption) in self.assumptions.iter().enumerate() {
            validate_assumption(&mut errors, assumption, &format!("assumptions[{index}]"));
        }
        require_sorted_unique(&mut errors, &self.assumptions, "assumptions", |item| {
            item.content_digest.clone()
        });
        for (index, evidence) in self.evidence.iter().enumerate() {
            validate_evidence_ref(&mut errors, evidence, &format!("evidence[{index}]"));
        }
        require_sorted_unique(&mut errors, &self.evidence, "evidence", |item| {
            (item.kind_uri.clone(), item.content_digest.clone())
        });
        if self.result == ClaimResult::Satisfied as i32 && self.evidence.is_empty() {
            errors.push(
                "evidence",
                "unsupported_satisfaction",
                "SATISFIED receipt requires exact evidence references",
            );
        }
        require_digest(
            &mut errors,
            &self.evidence_merkle_root,
            "evidenceMerkleRoot",
        );
        match evidence_merkle_root(&self.evidence) {
            Ok(computed) if computed != self.evidence_merkle_root => errors.push(
                "evidenceMerkleRoot",
                "digest_mismatch",
                format!("evidence references compute to {computed}"),
            ),
            Err(error) => errors.push(
                "evidenceMerkleRoot",
                "canonicalization_failed",
                error.to_string(),
            ),
            _ => {}
        }
        require_content_digest(&mut errors, self, "contentDigest");
        errors.finish()
    }
}

fn receipt_coverage_meets_claim(claim: &ClaimSpec, coverage: &Coverage) -> bool {
    let actual = Enumeration::try_from(coverage.enumeration).ok();
    let required = Enumeration::try_from(claim.required_enumeration).ok();
    let enumeration_ok = match required {
        Some(Enumeration::CompleteInScope) => actual == Some(Enumeration::CompleteInScope),
        Some(Enumeration::Partial) => {
            matches!(
                actual,
                Some(Enumeration::CompleteInScope | Enumeration::Partial)
            )
        }
        Some(Enumeration::Unknown) => actual.is_some(),
        _ => false,
    };
    enumeration_ok
        && claim
            .accepted_approximations
            .contains(&coverage.approximation)
        && (!claim.reject_proof_invalid_boundary
            || !coverage
                .boundaries
                .iter()
                .any(|boundary| boundary.consequence == BoundaryConsequence::ProofInvalid as i32))
}

impl Validate for ImpactReceipt {
    fn validate(&self) -> Result<(), ValidationErrors> {
        let mut errors = Errors::default();
        validate_schema(&mut errors, self.schema.as_ref(), "schema");
        require_text(&mut errors, &self.receipt_id, "receiptId");
        require_digest(&mut errors, &self.snapshot_id, "snapshotId");
        require_digest(&mut errors, &self.intent_digest, "intentDigest");
        require_digest(
            &mut errors,
            &self.obligation_graph_digest,
            "obligationGraphDigest",
        );
        require_sorted_unique(&mut errors, &self.affected, "affected", |item| {
            item.entity
                .as_ref()
                .map(|entity| (entity.adapter_namespace.clone(), entity.opaque_id.clone()))
        });
        for (index, affected) in self.affected.iter().enumerate() {
            let path = format!("affected[{index}]");
            if let Some(entity) = affected.entity.as_ref() {
                validate_entity(&mut errors, entity, &format!("{path}.entity"));
            } else {
                errors.required(false, &format!("{path}.entity"));
            }
            if ImpactClass::try_from(affected.impact_class).is_err()
                || affected.impact_class == ImpactClass::Unspecified as i32
            {
                errors.push(
                    format!("{path}.impactClass"),
                    "invalid_enum",
                    "specified impact class required",
                );
            }
            require_sorted_unique(
                &mut errors,
                &affected.relation_fact_ids,
                &format!("{path}.relationFactIds"),
                Clone::clone,
            );
        }
        if let Some(coverage) = self.coverage.as_ref() {
            validate_coverage(&mut errors, coverage, "coverage");
        } else {
            errors.required(false, "coverage");
        }
        require_sorted_unique(
            &mut errors,
            &self.unknown_boundaries,
            "unknownBoundaries",
            |item| item.boundary_id.clone(),
        );
        for (index, boundary) in self.unknown_boundaries.iter().enumerate() {
            validate_boundary(
                &mut errors,
                boundary,
                &format!("unknownBoundaries[{index}]"),
            );
        }
        errors.required(self.cost.is_some(), "cost");
        if ClaimResult::try_from(self.result).is_err()
            || self.result == ClaimResult::Unspecified as i32
        {
            errors.push("result", "invalid_enum", "specified impact result required");
        }
        if self.result == ClaimResult::Satisfied as i32
            && (!self.unknown_boundaries.is_empty()
                || self.coverage.as_ref().is_some_and(|coverage| {
                    coverage.enumeration != Enumeration::CompleteInScope as i32
                }))
        {
            errors.push(
                "result",
                "false_completeness",
                "SATISFIED impact receipt requires complete coverage and no unknown boundaries",
            );
        }
        require_content_digest(&mut errors, self, "contentDigest");
        errors.finish()
    }
}

impl Validate for EvidenceBundle {
    fn validate(&self) -> Result<(), ValidationErrors> {
        let mut errors = Errors::default();
        errors.append("snapshot", self.snapshot.validate());
        let source_digests = self
            .snapshot
            .sources
            .iter()
            .map(|source| (source.artifact_id.as_str(), source.content_digest.as_str()))
            .collect::<BTreeMap<_, _>>();
        require_sorted_unique(
            &mut errors,
            &self.capabilities,
            "capabilities",
            |decision| decision.content_digest.clone(),
        );
        let mut capability_digests = BTreeSet::new();
        for (index, capability) in self.capabilities.iter().enumerate() {
            errors.append(&format!("capabilities[{index}]"), capability.validate());
            if let Some(descriptor) = capability.descriptor.as_ref() {
                capability_digests.insert(descriptor.content_digest.as_str());
                if let Some(key) = descriptor.key.as_ref() {
                    let key_path = format!("capabilities[{index}].descriptor.key");
                    if key.snapshot_id != self.snapshot.snapshot_id {
                        errors.push(
                            format!("{key_path}.snapshotId"),
                            "snapshot_mismatch",
                            "capability is not bound to bundle snapshot",
                        );
                    }
                    if key.adapter != self.snapshot.adapter {
                        errors.push(
                            format!("{key_path}.adapter"),
                            "adapter_mismatch",
                            "capability adapter differs from snapshot adapter",
                        );
                    }
                    if self.snapshot.toolchain.as_ref().is_none_or(|toolchain| {
                        key.toolchain_digest != toolchain.distribution_digest
                    }) {
                        errors.push(
                            format!("{key_path}.toolchainDigest"),
                            "toolchain_mismatch",
                            "capability toolchain differs from snapshot toolchain",
                        );
                    }
                    if key.build_configuration_digest != self.snapshot.build_configuration_digest {
                        errors.push(
                            format!("{key_path}.buildConfigurationDigest"),
                            "configuration_mismatch",
                            "capability build configuration differs from snapshot",
                        );
                    }
                    if !self
                        .snapshot
                        .targets
                        .iter()
                        .any(|target| target.configuration_digest == key.target_digest)
                    {
                        errors.push(
                            format!("{key_path}.targetDigest"),
                            "target_mismatch",
                            "capability target is absent from snapshot targets",
                        );
                    }
                }
            }
            if capability
                .refusal
                .as_ref()
                .is_some_and(|refusal| refusal.snapshot_id != self.snapshot.snapshot_id)
            {
                errors.push(
                    format!("capabilities[{index}].refusal.snapshotId"),
                    "snapshot_mismatch",
                    "refusal is not bound to bundle snapshot",
                );
            }
        }
        require_sorted_unique(&mut errors, &self.batches, "batches", |batch| {
            (
                batch.capability_descriptor_digest.clone(),
                batch.content_digest.clone(),
            )
        });
        let mut fact_ids = BTreeSet::new();
        for (index, batch) in self.batches.iter().enumerate() {
            errors.append(&format!("batches[{index}]"), batch.validate());
            if batch.snapshot_id != self.snapshot.snapshot_id {
                errors.push(
                    format!("batches[{index}].snapshotId"),
                    "snapshot_mismatch",
                    "batch is not bound to bundle snapshot",
                );
            }
            if !capability_digests.contains(batch.capability_descriptor_digest.as_str()) {
                errors.push(
                    format!("batches[{index}].capabilityDescriptorDigest"),
                    "unknown_capability",
                    "batch references no supported descriptor in this bundle",
                );
            }
            let descriptor = self.capabilities.iter().find_map(|decision| {
                decision.descriptor.as_ref().filter(|descriptor| {
                    descriptor.content_digest == batch.capability_descriptor_digest
                })
            });
            for (entity_index, entity) in batch.entities.iter().enumerate() {
                validate_entity_snapshot_binding(
                    &mut errors,
                    entity,
                    &source_digests,
                    &format!("batches[{index}].entities[{entity_index}]"),
                );
            }
            for (occurrence_index, occurrence) in batch.occurrences.iter().enumerate() {
                validate_occurrence_snapshot_binding(
                    &mut errors,
                    occurrence,
                    &source_digests,
                    &format!("batches[{index}].occurrences[{occurrence_index}]"),
                );
            }
            for fact in &batch.facts {
                if !fact_ids.insert(fact.fact_id.as_str()) {
                    errors.push(
                        format!("batches[{index}].facts"),
                        "duplicate_fact_id",
                        format!("duplicate fact id {}", fact.fact_id),
                    );
                }
                if descriptor.is_some_and(|descriptor| {
                    fact.assertion
                        .as_ref()
                        .and_then(|assertion| assertion.relation.as_ref())
                        != descriptor
                            .key
                            .as_ref()
                            .and_then(|key| key.operation.as_ref())
                }) {
                    errors.push(
                        format!("batches[{index}].facts"),
                        "operation_mismatch",
                        "fact relation is not the operation certified by its capability",
                    );
                }
                if let Some(assertion) = fact.assertion.as_ref() {
                    for (operand_index, operand) in assertion.operands.iter().enumerate() {
                        match operand.value.as_ref() {
                            Some(operand::Value::Entity(entity)) => {
                                validate_entity_snapshot_binding(
                                    &mut errors,
                                    entity,
                                    &source_digests,
                                    &format!(
                                        "batches[{index}].facts.operand[{operand_index}].entity"
                                    ),
                                );
                            }
                            Some(operand::Value::Occurrence(occurrence)) => {
                                validate_occurrence_snapshot_binding(
                                    &mut errors,
                                    occurrence,
                                    &source_digests,
                                    &format!(
                                        "batches[{index}].facts.operand[{operand_index}].occurrence"
                                    ),
                                );
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        require_sorted_unique(
            &mut errors,
            &self.obligation_graphs,
            "obligationGraphs",
            |graph| graph.content_digest.clone(),
        );
        let mut graph_digests = BTreeSet::new();
        for (index, graph) in self.obligation_graphs.iter().enumerate() {
            errors.append(&format!("obligationGraphs[{index}]"), graph.validate());
            graph_digests.insert(graph.content_digest.as_str());
            if !capability_digests.contains(graph.closure_capability_digest.as_str()) {
                errors.push(
                    format!("obligationGraphs[{index}].closureCapabilityDigest"),
                    "unknown_capability",
                    "obligation closure capability is not registered in this bundle",
                );
            }
            if graph.snapshot_id != self.snapshot.snapshot_id {
                errors.push(
                    format!("obligationGraphs[{index}].snapshotId"),
                    "snapshot_mismatch",
                    "obligation graph is not bound to bundle snapshot",
                );
            }
            for obligation in &graph.obligations {
                for fact_id in &obligation.evidence_fact_ids {
                    if !fact_ids.contains(fact_id.as_str()) {
                        errors.push(
                            format!("obligationGraphs[{index}].obligations"),
                            "unknown_evidence_fact",
                            format!("unknown evidence fact {fact_id}"),
                        );
                    }
                }
            }
        }
        for (index, receipt) in self.verification_receipts.iter().enumerate() {
            errors.append(
                &format!("verificationReceipts[{index}]"),
                receipt.validate(),
            );
            if receipt.before_snapshot_id != self.snapshot.snapshot_id
                && receipt.after_snapshot_id.as_deref() != Some(&self.snapshot.snapshot_id)
            {
                errors.push(
                    format!("verificationReceipts[{index}].beforeSnapshotId"),
                    "snapshot_mismatch",
                    "receipt is unrelated to bundle snapshot",
                );
            }
            if !graph_digests.contains(receipt.obligation_graph_digest.as_str()) {
                errors.push(
                    format!("verificationReceipts[{index}].obligationGraphDigest"),
                    "unknown_obligation_graph",
                    "receipt references no obligation graph in bundle",
                );
            } else if receipt.result == ClaimResult::Satisfied as i32
                && self.obligation_graphs.iter().any(|graph| {
                    graph.content_digest == receipt.obligation_graph_digest
                        && graph.obligations.iter().any(|obligation| {
                            obligation.mandatory
                                && obligation.status != ObligationStatus::Satisfied as i32
                        })
                })
            {
                errors.push(
                    format!("verificationReceipts[{index}].result"),
                    "false_proof",
                    "SATISFIED receipt references an unsatisfied mandatory obligation",
                );
            }
        }
        for (index, receipt) in self.impact_receipts.iter().enumerate() {
            errors.append(&format!("impactReceipts[{index}]"), receipt.validate());
            if receipt.snapshot_id != self.snapshot.snapshot_id {
                errors.push(
                    format!("impactReceipts[{index}].snapshotId"),
                    "snapshot_mismatch",
                    "impact receipt is not bound to bundle snapshot",
                );
            }
            if !graph_digests.contains(receipt.obligation_graph_digest.as_str()) {
                errors.push(
                    format!("impactReceipts[{index}].obligationGraphDigest"),
                    "unknown_obligation_graph",
                    "receipt references no obligation graph in bundle",
                );
            }
        }
        errors.finish()
    }
}

fn validate_entity_snapshot_binding(
    errors: &mut Errors,
    entity: &EntityRef,
    sources: &BTreeMap<&str, &str>,
    path: &str,
) {
    if let Some(range) = entity.primary_definition.as_ref() {
        validate_range_snapshot_binding(
            errors,
            range,
            sources,
            &format!("{path}.primaryDefinition"),
        );
    }
}

fn validate_occurrence_snapshot_binding(
    errors: &mut Errors,
    occurrence: &Occurrence,
    sources: &BTreeMap<&str, &str>,
    path: &str,
) {
    if let Some(range) = occurrence.range.as_ref() {
        validate_range_snapshot_binding(errors, range, sources, &format!("{path}.range"));
    }
    if let Some(entity) = occurrence.entity.as_ref() {
        validate_entity_snapshot_binding(errors, entity, sources, &format!("{path}.entity"));
    }
}

fn validate_range_snapshot_binding(
    errors: &mut Errors,
    range: &SourceRange,
    sources: &BTreeMap<&str, &str>,
    path: &str,
) {
    match sources.get(range.artifact_id.as_str()) {
        None => errors.push(
            format!("{path}.artifactId"),
            "unknown_artifact",
            "range artifact is absent from snapshot source manifest",
        ),
        Some(content_digest) if *content_digest != range.artifact_content_digest => errors.push(
            format!("{path}.artifactContentDigest"),
            "artifact_digest_mismatch",
            "range content digest differs from snapshot source artifact",
        ),
        _ => {}
    }
}
