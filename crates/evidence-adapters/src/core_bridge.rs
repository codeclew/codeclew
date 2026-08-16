//! Translation from the ergonomic adapter JSON envelope into the frozen
//! language-neutral evidence protocol. The bridge is deliberately free of
//! language and benchmark dispatch: provider payloads remain opaque bytes.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use evidence_core::protocol::*;
use evidence_core::{
    EvidenceBundle, Validate, canonical_json_bytes, seal_content_digest, sha256_digest,
    validate_bundle,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{AdapterOutput, canonical_bytes, impact_query_specification};

const CORE_SCHEMA_BYTES: &[u8] = include_bytes!("../../../schemas/evidence_core.proto");
const ADAPTER_SCHEMA_BYTES: &[u8] = include_bytes!("../../../schemas/adapter_output.schema.json");

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CoreBindingSummary {
    pub schema: String,
    pub snapshot_id: String,
    pub bundle_digest: String,
    pub capability_count: usize,
    pub batch_count: usize,
    pub fact_count: usize,
    pub obligation_graph_count: usize,
    pub impact_receipt_count: usize,
    pub canonical_bundle_bytes: usize,
    pub translation_micros: u64,
    pub validation_micros: u64,
    pub digest_micros: u64,
    pub total_micros: u64,
}

/// Convert and validate the complete adapter result with the frozen typed
/// protocol. A raw JSON envelope is never sufficient authority on its own.
pub fn validate_core_binding(output: &AdapterOutput) -> Result<CoreBindingSummary> {
    let total_start = Instant::now();
    if output.schema != crate::ADAPTER_OUTPUT_SCHEMA {
        bail!("unsupported adapter output schema {}", output.schema);
    }
    output.verify_seal()?;
    if output
        .compiler_receipt
        .get("status")
        .and_then(Value::as_str)
        != Some("ACCEPTED")
        || output.compiler_receipt.get("grade").and_then(Value::as_str) != Some("COMPILER_CHECKED")
        || output
            .compiler_receipt
            .get("snapshotTreeDigest")
            .and_then(Value::as_str)
            != Some(output.snapshot_input.repository_tree_digest.as_str())
    {
        bail!("compiler receipt is not accepted and snapshot-bound");
    }
    let translation_start = Instant::now();
    let bundle = build_bundle(output)?;
    let translation_micros = translation_start.elapsed().as_micros() as u64;
    let validation_start = Instant::now();
    validate_bundle(&bundle).map_err(|error| {
        anyhow::anyhow!("typed evidence-core rejected adapter output: {error:?}")
    })?;
    let validation_micros = validation_start.elapsed().as_micros() as u64;
    let digest_start = Instant::now();
    let canonical_bundle = canonical_json_bytes(&bundle)?;
    let canonical_bundle_bytes = canonical_bundle.len();
    let bundle_digest = sha256_digest(canonical_bundle);
    let digest_micros = digest_start.elapsed().as_micros() as u64;
    Ok(CoreBindingSummary {
        schema: "codeclew.evidence-core-binding/0.1".to_owned(),
        snapshot_id: bundle.snapshot.snapshot_id.clone(),
        bundle_digest,
        capability_count: bundle.capabilities.len(),
        batch_count: bundle.batches.len(),
        fact_count: bundle.batches.iter().map(|batch| batch.facts.len()).sum(),
        obligation_graph_count: bundle.obligation_graphs.len(),
        impact_receipt_count: bundle.impact_receipts.len(),
        canonical_bundle_bytes,
        translation_micros,
        validation_micros,
        digest_micros,
        total_micros: total_start.elapsed().as_micros() as u64,
    })
}

fn build_bundle(output: &AdapterOutput) -> Result<EvidenceBundle> {
    let snapshot = snapshot(output)?;
    let boundaries = boundaries(output)?;
    let scope = workspace_scope(output)?;
    let output_ref = EvidenceRef {
        kind_uri: "codeclew.evidence/adapter-output/1".to_owned(),
        content_digest: output.output_digest.clone(),
    };

    let mut descriptors = BTreeMap::<String, CapabilityDescriptor>::new();
    let mut decisions = Vec::new();
    for raw in &output.capability_descriptors {
        let descriptor = capability(output, &snapshot, raw, &scope, &boundaries)?;
        let operation_uri = descriptor
            .key
            .as_ref()
            .and_then(|key| key.operation.as_ref())
            .map(|operation| operation.uri.clone())
            .context("typed capability has no operation")?;
        if descriptors
            .insert(operation_uri.clone(), descriptor.clone())
            .is_some()
        {
            bail!("adapter publishes duplicate operation {operation_uri}");
        }
        let mut decision = CapabilityDecision {
            status: SupportStatus::Supported as i32,
            descriptor: Some(descriptor),
            refusal: None,
            content_digest: String::new(),
        };
        seal_content_digest(&mut decision)?;
        decisions.push(decision);
    }

    let impact_descriptor = impact_capability(output, &snapshot, &scope, &boundaries)?;
    let impact_descriptor_digest = impact_descriptor.content_digest.clone();
    let mut impact_decision = CapabilityDecision {
        status: SupportStatus::Supported as i32,
        descriptor: Some(impact_descriptor),
        refusal: None,
        content_digest: String::new(),
    };
    seal_content_digest(&mut impact_decision)?;
    decisions.push(impact_decision);
    decisions.sort_by(|left, right| left.content_digest.cmp(&right.content_digest));

    let entities = entities(output)?;
    let entity_index = entities
        .iter()
        .map(|entity| (entity.opaque_id.clone(), entity.clone()))
        .collect::<BTreeMap<_, _>>();
    let occurrences = occurrences(output, &entity_index, &output_ref)?;
    let fact_boundaries = descriptors
        .iter()
        .map(|(operation, descriptor)| {
            Ok((
                operation.clone(),
                compact_fact_boundaries(descriptor, &output_ref)?,
            ))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    let mut grouped = BTreeMap::<String, Vec<EvidenceFact>>::new();
    for raw in &output.facts {
        let operation_uri = text(raw, "relation")?.to_owned();
        let descriptor = descriptors
            .get(&operation_uri)
            .with_context(|| format!("fact uses undeclared operation {operation_uri}"))?;
        let compact_boundaries = &fact_boundaries[&operation_uri];
        grouped.entry(operation_uri).or_default().push(fact(
            &snapshot,
            raw,
            descriptor,
            &scope,
            compact_boundaries,
            &output_ref,
        )?);
    }

    let mut batches = Vec::new();
    let mut attach_inventory = true;
    for (operation_uri, mut facts) in grouped {
        facts.sort_by(|left, right| left.fact_id.cmp(&right.fact_id));
        let descriptor = &descriptors[&operation_uri];
        let mut batch = EvidenceBatch {
            schema: Some(schema("codeclew.schema/evidence-batch/1")),
            snapshot_id: snapshot.snapshot_id.clone(),
            capability_descriptor_digest: descriptor.content_digest.clone(),
            entities: if attach_inventory {
                entities.clone()
            } else {
                vec![]
            },
            occurrences: if attach_inventory {
                occurrences.clone()
            } else {
                vec![]
            },
            facts,
            artifacts: vec![output_ref.clone()],
            content_digest: String::new(),
        };
        attach_inventory = false;
        seal_content_digest(&mut batch)?;
        batches.push(batch);
    }
    batches.sort_by(|left, right| {
        (&left.capability_descriptor_digest, &left.content_digest)
            .cmp(&(&right.capability_descriptor_digest, &right.content_digest))
    });

    let (graph, impact_receipt) = impact_evidence(
        output,
        &snapshot,
        &entity_index,
        &scope,
        &boundaries,
        &impact_descriptor_digest,
        &output_ref,
    )?;
    Ok(EvidenceBundle {
        snapshot,
        capabilities: decisions,
        batches,
        obligation_graphs: vec![graph],
        verification_receipts: vec![],
        impact_receipts: vec![impact_receipt],
    })
}

fn snapshot(output: &AdapterOutput) -> Result<WorkspaceAnalysisSnapshot> {
    let raw_toolchain = &output.snapshot_input.toolchain;
    let tool_uri = text(raw_toolchain, "toolUri")?;
    let version = text(raw_toolchain, "version")?;
    let distribution_digest = text(raw_toolchain, "distributionDigest")?;
    let sources = output
        .snapshot_input
        .sources
        .iter()
        .map(|source| {
            Ok(SourceArtifact {
                artifact_id: source.artifact_id.clone(),
                normalized_path: source.normalized_path.clone(),
                content_digest: source.content_digest.clone(),
                origin: artifact_origin(&source.origin)? as i32,
                generator_id: String::new(),
                generated_from: vec![],
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let mut targets = output
        .snapshot_input
        .targets
        .iter()
        .map(|raw| {
            let enabled_features = raw
                .get("enabledFeatures")
                .and_then(Value::as_array)
                .context("target enabledFeatures must be an array")?
                .iter()
                .map(|value| {
                    value
                        .as_str()
                        .map(ToOwned::to_owned)
                        .context("enabled feature must be a string")
                })
                .collect::<Result<Vec<_>>>()?;
            let compiler_flags = raw
                .get("compilerFlags")
                .and_then(Value::as_array)
                .context("target compilerFlags must be an ordered string array")?
                .iter()
                .map(|value| {
                    value
                        .as_str()
                        .map(ToOwned::to_owned)
                        .context("compiler flag must be a string")
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(BuildTarget {
                target_id: text(raw, "targetId")?.to_owned(),
                configuration_digest: text(raw, "configurationDigest")?.to_owned(),
                enabled_features,
                platform: text(raw, "platform")?.to_owned(),
                compiler_flags,
                language_payload: Some(payload("codeclew.schema/adapter-target/1", raw)?),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    targets.sort_by(|left, right| left.target_id.cmp(&right.target_id));
    let mut environment = output
        .snapshot_input
        .relevant_environment
        .iter()
        .map(|raw| {
            Ok(KeyValue {
                key: text(raw, "key")?.to_owned(),
                value: text(raw, "value")?.to_owned(),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    environment.sort_by(|left, right| left.key.cmp(&right.key));
    let mut snapshot = WorkspaceAnalysisSnapshot {
        schema: Some(schema("codeclew.schema/workspace-analysis-snapshot/1")),
        snapshot_id: String::new(),
        repository_tree_digest: output.snapshot_input.repository_tree_digest.clone(),
        vcs_revision: output.snapshot_input.vcs_revision.clone(),
        dirty: output.snapshot_input.dirty,
        sources,
        build_system_uri: output.snapshot_input.build_system_uri.clone(),
        build_model_digest: output.snapshot_input.build_model_digest.clone(),
        build_configuration_digest: output.snapshot_input.build_configuration_digest.clone(),
        dependency_graph_digest: output.snapshot_input.dependency_graph_digest.clone(),
        toolchain: Some(ToolIdentity {
            tool_uri: tool_uri.to_owned(),
            version: version.to_owned(),
            distribution_digest: distribution_digest.to_owned(),
            plugins: vec![],
            language_payload: Some(payload(
                "codeclew.schema/adapter-toolchain/1",
                raw_toolchain,
            )?),
        }),
        targets,
        relevant_environment: environment,
        generated_sources_manifest_digest: output
            .snapshot_input
            .generated_sources_manifest_digest
            .clone(),
        adapter: Some(AdapterIdentity {
            adapter_id: output.adapter.adapter_id.clone(),
            version: output.adapter.version.clone(),
            binary_digest: output.adapter.binary_digest.clone(),
        }),
        metadata: None,
    };
    snapshot.seal_snapshot_id()?;
    snapshot
        .validate()
        .map_err(|error| anyhow::anyhow!("typed snapshot rejected: {error:?}"))?;
    Ok(snapshot)
}

fn capability(
    output: &AdapterOutput,
    snapshot: &WorkspaceAnalysisSnapshot,
    raw: &Value,
    scope: &Scope,
    boundaries: &[Boundary],
) -> Result<CapabilityDescriptor> {
    if text(raw, "support")? != "SUPPORTED" {
        bail!("unsupported capability must be represented by a typed refusal");
    }
    for (field, expected) in [
        ("languageId", output.adapter.language_id.as_str()),
        ("adapterId", output.adapter.adapter_id.as_str()),
        ("adapterVersion", output.adapter.version.as_str()),
    ] {
        if text(raw, field)? != expected {
            bail!("capability {field} differs from adapter identity");
        }
    }
    let target_digest = text(raw, "targetDigest")?;
    if !snapshot
        .targets
        .iter()
        .any(|target| target.configuration_digest == target_digest)
    {
        bail!("capability targetDigest is not present in snapshot targets");
    }
    let operation = OperationRef {
        uri: text(raw, "operationUri")?.to_owned(),
        version: text(raw, "operationVersion")?.to_owned(),
        specification_digest: text(raw, "operationSpecificationDigest")?.to_owned(),
    };
    let declared_boundary_kinds = string_set(raw, "knownBoundaryKinds")?;
    let operation_boundaries = boundaries
        .iter()
        .filter(|boundary| {
            declared_boundary_kinds
                .binary_search(&boundary.kind_uri)
                .is_ok()
        })
        .cloned()
        .collect::<Vec<_>>();
    let enumeration = enumeration(text(raw, "guaranteedEnumeration")?)?;
    let approximation = approximation(text(raw, "approximation")?)?;
    let coverage = coverage(
        scope,
        enumeration,
        approximation,
        operation_boundaries.clone(),
    )?;
    let mut descriptor = CapabilityDescriptor {
        schema: Some(schema("codeclew.schema/capability-descriptor/1")),
        key: Some(CapabilityKey {
            language_id: output.adapter.language_id.clone(),
            adapter: snapshot.adapter.clone(),
            snapshot_id: snapshot.snapshot_id.clone(),
            toolchain_digest: text(raw, "toolchainDigest")?.to_owned(),
            build_configuration_digest: text(raw, "buildConfigurationDigest")?.to_owned(),
            target_digest: target_digest.to_owned(),
            operation: Some(operation),
            grade: grade(text(raw, "grade")?)? as i32,
        }),
        input_domain_uris: vec!["codeclew.domain/source/1".to_owned()],
        output_schema: Some(schema("codeclew.schema/evidence-batch/1")),
        guaranteed_coverage: Some(coverage),
        required_capability_digests: vec![],
        known_boundary_kind_uris: declared_boundary_kinds,
        assumptions: vec![],
        supported_contour: vec![scope.clone()],
        unsupported_contour: vec![],
        cost_class_uri: text(raw, "costClass")?.to_owned(),
        content_digest: String::new(),
    };
    seal_content_digest(&mut descriptor)?;
    Ok(descriptor)
}

fn impact_capability(
    output: &AdapterOutput,
    snapshot: &WorkspaceAnalysisSnapshot,
    scope: &Scope,
    boundaries: &[Boundary],
) -> Result<CapabilityDescriptor> {
    let target = snapshot.targets.first().context("snapshot has no target")?;
    let operation = OperationRef {
        uri: "codeclew.operation/change-impact/1".to_owned(),
        version: "1".to_owned(),
        specification_digest: sha256_digest(
            output
                .impact
                .get("closureSpecification")
                .and_then(Value::as_str)
                .unwrap_or("codeclew.impact/unknown")
                .as_bytes(),
        ),
    };
    let status = output
        .impact
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("UNKNOWN");
    let enumeration = if status == "COMPLETE_IN_SCOPE" {
        Enumeration::CompleteInScope
    } else {
        Enumeration::Partial
    };
    let impact_boundaries = effective_impact_boundaries(output, boundaries)?;
    let mut descriptor = CapabilityDescriptor {
        schema: Some(schema("codeclew.schema/capability-descriptor/1")),
        key: Some(CapabilityKey {
            language_id: output.adapter.language_id.clone(),
            adapter: snapshot.adapter.clone(),
            snapshot_id: snapshot.snapshot_id.clone(),
            toolchain_digest: snapshot
                .toolchain
                .as_ref()
                .unwrap()
                .distribution_digest
                .clone(),
            build_configuration_digest: snapshot.build_configuration_digest.clone(),
            target_digest: target.configuration_digest.clone(),
            operation: Some(operation),
            grade: EvidenceGrade::StaticallyApproximated as i32,
        }),
        input_domain_uris: vec!["codeclew.domain/evidence-graph/1".to_owned()],
        output_schema: Some(schema("codeclew.schema/impact-receipt/1")),
        guaranteed_coverage: Some(coverage(
            scope,
            enumeration,
            Approximation::SoundOver,
            impact_boundaries.clone(),
        )?),
        required_capability_digests: vec![],
        known_boundary_kind_uris: impact_boundaries
            .iter()
            .map(|boundary| boundary.kind_uri.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
        assumptions: vec![],
        supported_contour: vec![scope.clone()],
        unsupported_contour: vec![],
        cost_class_uri: "codeclew.cost/bounded-graph-query/1".to_owned(),
        content_digest: String::new(),
    };
    seal_content_digest(&mut descriptor)?;
    Ok(descriptor)
}

fn entities(output: &AdapterOutput) -> Result<Vec<EntityRef>> {
    let mut entities = output
        .entities
        .iter()
        .map(|raw| {
            Ok(EntityRef {
                adapter_namespace: text(raw, "adapterNamespace")?.to_owned(),
                opaque_id: text(raw, "opaqueId")?.to_owned(),
                resolution: entity_resolution(text(raw, "resolution")?)? as i32,
                coarse_kind: coarse_kind(text(raw, "coarseKind")?)? as i32,
                display_name: raw
                    .get("displayName")
                    .and_then(Value::as_str)
                    .unwrap_or(text(raw, "opaqueId")?)
                    .to_owned(),
                primary_definition: optional_range(raw.get("primaryDefinition"))?,
                language_payload: Some(payload("codeclew.schema/adapter-entity/1", raw)?),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    entities.sort_by(|left, right| {
        (&left.adapter_namespace, &left.opaque_id)
            .cmp(&(&right.adapter_namespace, &right.opaque_id))
    });
    Ok(entities)
}

fn occurrences(
    output: &AdapterOutput,
    entities: &BTreeMap<String, EntityRef>,
    output_ref: &EvidenceRef,
) -> Result<Vec<Occurrence>> {
    let mut values = output
        .occurrences
        .iter()
        .map(|raw| {
            let fallback_id = format!("unresolved-occurrence:{}", text(raw, "occurrenceId")?);
            let entity_id = raw
                .get("entityId")
                .and_then(Value::as_str)
                .unwrap_or(&fallback_id);
            let entity = entities
                .get(entity_id)
                .cloned()
                .unwrap_or_else(|| unresolved_entity(output, entity_id));
            let mut occurrence = Occurrence {
                occurrence_id: text(raw, "occurrenceId")?.to_owned(),
                range: optional_range(raw.get("range"))?,
                roles: vec![occurrence_role(text(raw, "role")?)? as i32],
                origin: occurrence_origin(text(raw, "origin")?)? as i32,
                entity: Some(entity),
                evidence: vec![output_ref.clone()],
                content_digest: String::new(),
            };
            if occurrence.range.is_none() {
                bail!(
                    "occurrence {} has no source range",
                    occurrence.occurrence_id
                );
            }
            seal_content_digest(&mut occurrence)?;
            Ok(occurrence)
        })
        .collect::<Result<Vec<_>>>()?;
    values.sort_by(|left, right| left.occurrence_id.cmp(&right.occurrence_id));
    Ok(values)
}

#[allow(clippy::too_many_arguments)]
fn fact(
    snapshot: &WorkspaceAnalysisSnapshot,
    raw: &Value,
    descriptor: &CapabilityDescriptor,
    scope: &Scope,
    selected_boundaries: &[Boundary],
    output_ref: &EvidenceRef,
) -> Result<EvidenceFact> {
    let key = descriptor.key.as_ref().unwrap();
    if grade(text(raw, "grade")?)? as i32 != key.grade {
        bail!("fact evidence grade differs from its capability descriptor");
    }
    let operation = key.operation.clone().unwrap();
    let enumeration = enumeration(text(raw, "enumeration")?)?;
    let approximation = descriptor
        .guaranteed_coverage
        .as_ref()
        .and_then(|coverage| Approximation::try_from(coverage.approximation).ok())
        .unwrap_or(Approximation::Heuristic);
    let mut value = EvidenceFact {
        schema: Some(schema("codeclew.schema/evidence-fact/1")),
        fact_id: text(raw, "factId")?.to_owned(),
        snapshot_id: snapshot.snapshot_id.clone(),
        capability_descriptor_digest: descriptor.content_digest.clone(),
        assertion: Some(RelationAssertion {
            relation: Some(operation),
            operands: vec![
                Operand {
                    name: "owner".to_owned(),
                    value: Some(operand::Value::Identity(text(raw, "owner")?.to_owned())),
                },
                Operand {
                    name: "target".to_owned(),
                    value: Some(operand::Value::Identity(text(raw, "target")?.to_owned())),
                },
            ],
            truth: truth(text(raw, "truth")?)? as i32,
            language_payload: Some(payload(
                "codeclew.schema/adapter-relation-fact/1",
                raw.get("providerPayload").unwrap_or(raw),
            )?),
        }),
        provenance: vec![output_ref.clone()],
        coverage: Some(coverage(
            scope,
            enumeration,
            approximation,
            selected_boundaries.to_vec(),
        )?),
        content_digest: String::new(),
    };
    seal_content_digest(&mut value)?;
    Ok(value)
}

/// Facts bind the exact adapter output as provenance, so repeating every
/// concrete workspace boundary in every fact would add O(facts * boundaries)
/// bytes without adding authority. A compact, content-addressed set boundary
/// preserves the exact set identity and strongest consequence; capability
/// coverage and the adapter envelope retain the individual boundary records.
fn compact_fact_boundaries(
    descriptor: &CapabilityDescriptor,
    output_ref: &EvidenceRef,
) -> Result<Vec<Boundary>> {
    let Some(coverage) = descriptor.guaranteed_coverage.as_ref() else {
        return Ok(vec![]);
    };
    if coverage.boundaries.is_empty() {
        return Ok(vec![]);
    }
    let members = coverage
        .boundaries
        .iter()
        .map(|boundary| boundary.content_digest.clone())
        .collect::<Vec<_>>();
    let consequence =
        if coverage
            .boundaries
            .iter()
            .any(|boundary| boundary.consequence == BoundaryConsequence::ProofInvalid as i32)
        {
            BoundaryConsequence::ProofInvalid
        } else if coverage.boundaries.iter().any(|boundary| {
            boundary.consequence == BoundaryConsequence::EnumerationIncomplete as i32
        }) {
            BoundaryConsequence::EnumerationIncomplete
        } else {
            BoundaryConsequence::LocalOnly
        };
    let set_digest = sha256_digest(canonical_json_bytes(&members)?);
    let mut boundary = Boundary {
        boundary_id: set_digest.clone(),
        kind_uri: "codeclew.boundary/adapter-coverage-set/1".to_owned(),
        origin: "null".to_owned(),
        consequence: consequence as i32,
        details: Some(payload(
            "codeclew.schema/boundary-set/1",
            &json!({
                "memberCount":members.len(),
                "memberSetDigest":set_digest,
            }),
        )?),
        evidence: vec![output_ref.clone()],
        content_digest: String::new(),
    };
    seal_content_digest(&mut boundary)?;
    Ok(vec![boundary])
}

#[allow(clippy::too_many_arguments)]
fn impact_evidence(
    output: &AdapterOutput,
    snapshot: &WorkspaceAnalysisSnapshot,
    entities: &BTreeMap<String, EntityRef>,
    scope: &Scope,
    boundaries: &[Boundary],
    impact_capability_digest: &str,
    output_ref: &EvidenceRef,
) -> Result<(ObligationGraph, ImpactReceipt)> {
    let impact_boundaries = effective_impact_boundaries(output, boundaries)?;
    let seed = output.impact.get("seedEntity").and_then(Value::as_str);
    let intent_digest = sha256_digest(canonical_json_bytes(&json!({
        "seedEntity":seed,
        "maxDepth":output.impact.get("maxDepth"),
        "maxEntities":output.impact.get("maxEntities"),
        "querySpecification":output.impact.get("querySpecification"),
    }))?);
    let closure_specification = output
        .impact
        .get("closureSpecification")
        .and_then(Value::as_str)
        .unwrap_or("codeclew.impact/unknown");
    let boundary_assessment_digest = output
        .impact
        .get("boundaryAssessment")
        .map(canonical_json_bytes)
        .transpose()?
        .map(sha256_digest);
    let mut obligations = output
        .impact
        .get("mandatoryObligations")
        .and_then(Value::as_array)
        .context("impact mandatoryObligations must be an array")?
        .iter()
        .map(|raw| {
            let provider_status = text(raw, "status")?;
            let obligation_kind = text(raw, "kind")?;
            let evidence_ids = raw
                .get("evidenceFactIds")
                .and_then(Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .map(|value| {
                            value
                                .as_str()
                                .map(ToOwned::to_owned)
                                .context("evidenceFactId must be a string")
                        })
                        .collect::<Result<Vec<_>>>()
                })
                .transpose()?
                .unwrap_or_default();
            let has_bound_closure_attestation = obligation_kind
                == "codeclew.obligation/impact-closure-completeness/1"
                && impact_boundaries.is_empty()
                && raw
                    .pointer("/providerPayload/boundaryAssessmentDigest")
                    .and_then(Value::as_str)
                    == boundary_assessment_digest.as_deref();
            let status = if provider_status == "SATISFIED"
                && evidence_ids.is_empty()
                && !has_bound_closure_attestation
            {
                ObligationStatus::Unknown
            } else {
                obligation_status(provider_status)?
            };
            let mut obligation = Obligation {
                obligation_id: text(raw, "id")?.to_owned(),
                origin_intent_digest: intent_digest.clone(),
                required_operation: Some(OperationRef {
                    uri: obligation_kind.to_owned(),
                    version: "1".to_owned(),
                    specification_digest: sha256_digest(obligation_kind.as_bytes()),
                }),
                scope: Some(scope.clone()),
                precondition: None,
                postcondition: Some(payload("codeclew.schema/adapter-obligation/1", raw)?),
                accepted_grades: vec![EvidenceGrade::StaticallyApproximated as i32],
                dependency_ids: vec![],
                mandatory: raw
                    .get("mandatory")
                    .and_then(Value::as_bool)
                    .unwrap_or(true),
                status: status as i32,
                evidence_fact_ids: evidence_ids,
                unknown_reason: if matches!(
                    status,
                    ObligationStatus::Unknown | ObligationStatus::Unsupported
                ) {
                    raw.get("reason")
                        .and_then(Value::as_str)
                        .unwrap_or("PROVIDER_DID_NOT_SUPPLY_DISCHARGING_EVIDENCE")
                        .to_owned()
                } else {
                    String::new()
                },
                content_digest: String::new(),
            };
            obligation.evidence_fact_ids.sort();
            seal_content_digest(&mut obligation)?;
            Ok(obligation)
        })
        .collect::<Result<Vec<_>>>()?;
    if obligations.is_empty() {
        let mut obligation = Obligation {
            obligation_id: "impact-completeness".to_owned(),
            origin_intent_digest: intent_digest.clone(),
            required_operation: Some(OperationRef {
                uri: "codeclew.obligation/impact-completeness/1".to_owned(),
                version: "1".to_owned(),
                specification_digest: sha256_digest(b"codeclew.obligation/impact-completeness/1"),
            }),
            scope: Some(scope.clone()),
            precondition: None,
            postcondition: None,
            accepted_grades: vec![EvidenceGrade::StaticallyApproximated as i32],
            dependency_ids: vec![],
            mandatory: true,
            status: ObligationStatus::Unknown as i32,
            evidence_fact_ids: vec![],
            unknown_reason: "PROVIDER_RETURNED_NO_EXPLICIT_CLOSURE".to_owned(),
            content_digest: String::new(),
        };
        seal_content_digest(&mut obligation)?;
        obligations.push(obligation);
    }
    obligations.sort_by(|left, right| left.obligation_id.cmp(&right.obligation_id));
    let mut graph = ObligationGraph {
        schema: Some(schema("codeclew.schema/obligation-graph/1")),
        snapshot_id: snapshot.snapshot_id.clone(),
        intent_digest: intent_digest.clone(),
        closure_capability_digest: impact_capability_digest.to_owned(),
        closure_specification_digest: sha256_digest(closure_specification.as_bytes()),
        obligations,
        content_digest: String::new(),
    };
    seal_content_digest(&mut graph)?;

    let mut affected = output
        .impact
        .get("affected")
        .and_then(Value::as_array)
        .context("impact affected must be an array")?
        .iter()
        .map(|raw| {
            let entity_id = text(raw, "entityId")?;
            let mut fact_ids = output
                .impact
                .get("paths")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|path| path.get("to").and_then(Value::as_str) == Some(entity_id))
                .filter_map(|path| {
                    path.get("factId")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned)
                })
                .collect::<Vec<_>>();
            fact_ids.sort();
            fact_ids.dedup();
            Ok(ImpactedEntity {
                entity: Some(
                    entities
                        .get(entity_id)
                        .cloned()
                        .unwrap_or_else(|| unresolved_entity(output, entity_id)),
                ),
                impact_class: impact_class(text(raw, "impactClass")?)? as i32,
                relation_fact_ids: fact_ids,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    affected.sort_by(|left, right| {
        let left = left.entity.as_ref().unwrap();
        let right = right.entity.as_ref().unwrap();
        (&left.adapter_namespace, &left.opaque_id)
            .cmp(&(&right.adapter_namespace, &right.opaque_id))
    });
    let status = output
        .impact
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("UNKNOWN");
    let result = if status == "COMPLETE_IN_SCOPE"
        && impact_boundaries.is_empty()
        && graph
            .obligations
            .iter()
            .filter(|obligation| obligation.mandatory)
            .all(|obligation| obligation.status == ObligationStatus::Satisfied as i32)
    {
        ClaimResult::Satisfied
    } else {
        ClaimResult::Unknown
    };
    // Enumeration and claim truth are independent. A provider may enumerate
    // the declared contour completely while an obligation remains UNKNOWN.
    let enumeration = if status == "COMPLETE_IN_SCOPE" {
        Enumeration::CompleteInScope
    } else {
        Enumeration::Partial
    };
    let mut receipt = ImpactReceipt {
        schema: Some(schema("codeclew.schema/impact-receipt/1")),
        receipt_id: format!("impact:{}", &intent_digest[7..23]),
        snapshot_id: snapshot.snapshot_id.clone(),
        intent_digest,
        affected,
        obligation_graph_digest: graph.content_digest.clone(),
        coverage: Some(coverage(
            scope,
            enumeration,
            Approximation::SoundOver,
            impact_boundaries.clone(),
        )?),
        unknown_boundaries: impact_boundaries,
        cost: Some(CostTelemetry {
            build_discovery_micros: output.cost.build_discovery_micros,
            cold_index_micros: output.cost.cold_index_micros,
            warm_index_micros: output.cost.warm_index_micros,
            adapter_micros: output.cost.adapter_micros,
            query_micros: output.cost.query_micros,
            stored_fact_bytes: output.cost.stored_fact_bytes,
            model_visible_source_bytes: output.cost.model_visible_source_bytes,
        }),
        result: result as i32,
        content_digest: String::new(),
    };
    let _ = output_ref;
    seal_content_digest(&mut receipt)?;
    Ok((graph, receipt))
}

fn effective_impact_boundaries(
    output: &AdapterOutput,
    boundaries: &[Boundary],
) -> Result<Vec<Boundary>> {
    let Some(raw_impact_boundaries) = output.impact.get("boundaries").and_then(Value::as_array)
    else {
        // Old adapters did not distinguish project inventory from query
        // closure. Retain their full fail-closed boundary set.
        return Ok(boundaries.to_vec());
    };
    if output.impact.get("querySpecification") != Some(&impact_query_specification()) {
        // Legacy or malformed query contracts have no authority to narrow
        // project-wide uncertainty. Preserve the full boundary inventory.
        if output.impact.get("status").and_then(Value::as_str) == Some("COMPLETE_IN_SCOPE")
            && !boundaries.is_empty()
        {
            bail!("complete impact has no scoped authority over project boundaries");
        }
        return Ok(boundaries.to_vec());
    }
    let raw_project_by_id = output
        .boundaries
        .iter()
        .filter_map(|raw| {
            raw.get("boundaryId")
                .and_then(Value::as_str)
                .map(|id| (id, raw))
        })
        .collect::<BTreeMap<_, _>>();
    let project_by_id = boundaries
        .iter()
        .map(|boundary| (boundary.boundary_id.as_str(), boundary))
        .collect::<BTreeMap<_, _>>();
    let mut selected = Vec::new();
    let mut seen = BTreeSet::new();
    for raw in raw_impact_boundaries {
        let id = text(raw, "boundaryId")?;
        if !seen.insert(id.to_owned()) {
            bail!("impact contains a duplicate query boundary {id}");
        }
        if let Some(project_raw) = raw_project_by_id.get(id) {
            if *project_raw != raw {
                bail!("impact query boundary differs from project boundary {id}");
            }
            selected.push(
                (*project_by_id
                    .get(id)
                    .context("impact query boundary has no typed project boundary")?)
                .clone(),
            );
            continue;
        }
        let kind = text(raw, "kindUri")?;
        if !matches!(
            kind,
            "codeclew.boundary/budget-max-entities/1" | "codeclew.boundary/no-seed-entity/1"
        ) {
            bail!("impact contains an unbound query boundary {id}");
        }
        selected.push(boundary(raw)?);
    }
    selected.sort_by(|left, right| left.boundary_id.cmp(&right.boundary_id));
    if output.impact.get("status").and_then(Value::as_str) == Some("COMPLETE_IN_SCOPE") {
        if !selected.is_empty() {
            bail!("complete impact contains query-relevant boundaries");
        }
        return Ok(selected);
    }
    if !selected.is_empty() {
        return Ok(selected);
    }
    let reason = output
        .impact
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or("PROVIDER_DID_NOT_DECLARE_COMPLETE_IMPACT");
    let mut boundary = Boundary {
        boundary_id: sha256_digest(canonical_json_bytes(&json!({
            "kind":"impact-not-complete",
            "reason":reason,
            "status":output.impact.get("status"),
        }))?),
        kind_uri: "codeclew.boundary/impact-not-complete/1".to_owned(),
        origin: "null".to_owned(),
        consequence: BoundaryConsequence::EnumerationIncomplete as i32,
        details: Some(payload(
            "codeclew.schema/impact-boundary/1",
            &json!({"reason":reason}),
        )?),
        evidence: vec![],
        content_digest: String::new(),
    };
    seal_content_digest(&mut boundary)?;
    Ok(vec![boundary])
}

fn boundaries(output: &AdapterOutput) -> Result<Vec<Boundary>> {
    let mut values = output
        .boundaries
        .iter()
        .map(boundary)
        .collect::<Result<Vec<_>>>()?;
    values.sort_by(|left, right| left.boundary_id.cmp(&right.boundary_id));
    Ok(values)
}

fn boundary(raw: &Value) -> Result<Boundary> {
    let mut boundary = Boundary {
        boundary_id: text(raw, "boundaryId")?.to_owned(),
        kind_uri: text(raw, "kindUri")?.to_owned(),
        origin: canonical_bytes(raw.get("origin").unwrap_or(&Value::Null))
            .map(String::from_utf8)?
            .unwrap_or_else(|_| "null".to_owned()),
        consequence: boundary_consequence(text(raw, "consequence")?)? as i32,
        details: Some(payload(
            "codeclew.schema/adapter-boundary/1",
            raw.get("details").unwrap_or(raw),
        )?),
        evidence: vec![],
        content_digest: String::new(),
    };
    seal_content_digest(&mut boundary)?;
    Ok(boundary)
}

fn coverage(
    scope: &Scope,
    enumeration: Enumeration,
    approximation: Approximation,
    mut boundaries: Vec<Boundary>,
) -> Result<Coverage> {
    boundaries.sort_by(|left, right| left.boundary_id.cmp(&right.boundary_id));
    let mut value = Coverage {
        scopes: vec![scope.clone()],
        enumeration: enumeration as i32,
        approximation: approximation as i32,
        boundaries,
        assumptions: vec![],
        content_digest: String::new(),
    };
    seal_content_digest(&mut value)?;
    Ok(value)
}

fn workspace_scope(output: &AdapterOutput) -> Result<Scope> {
    let mut value = Scope {
        scope_uri: "codeclew.scope/workspace-targets/1".to_owned(),
        selector: Some(payload(
            "codeclew.schema/workspace-scope/1",
            &json!({
                "repositoryTreeDigest":output.snapshot_input.repository_tree_digest,
                "targets":output.snapshot_input.targets,
            }),
        )?),
        content_digest: String::new(),
    };
    seal_content_digest(&mut value)?;
    Ok(value)
}

fn schema(uri: &str) -> SchemaRef {
    SchemaRef {
        uri: uri.to_owned(),
        major: 1,
        minor: 0,
        specification_digest: sha256_digest(CORE_SCHEMA_BYTES),
    }
}

fn payload(uri: &str, value: &Value) -> Result<TypedPayload> {
    let bytes = canonical_bytes(value)?;
    Ok(TypedPayload {
        schema: Some(SchemaRef {
            uri: uri.to_owned(),
            major: 1,
            minor: 0,
            specification_digest: sha256_digest(ADAPTER_SCHEMA_BYTES),
        }),
        media_type: "application/json".to_owned(),
        content_digest: sha256_digest(&bytes),
        canonical_bytes: bytes,
    })
}

fn text<'a>(value: &'a Value, field: &str) -> Result<&'a str> {
    value
        .get(field)
        .and_then(Value::as_str)
        .with_context(|| format!("adapter value misses string {field}"))
}

fn optional_range(value: Option<&Value>) -> Result<Option<SourceRange>> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    Ok(Some(SourceRange {
        artifact_id: text(value, "artifactId")?.to_owned(),
        artifact_content_digest: text(value, "artifactContentDigest")?.to_owned(),
        start_byte: value
            .get("startByte")
            .and_then(Value::as_u64)
            .context("range startByte")?,
        end_byte: value
            .get("endByte")
            .and_then(Value::as_u64)
            .context("range endByte")?,
    }))
}

fn unresolved_entity(output: &AdapterOutput, opaque_id: &str) -> EntityRef {
    EntityRef {
        adapter_namespace: format!("{}/unresolved", output.adapter.adapter_id),
        opaque_id: opaque_id.to_owned(),
        resolution: EntityResolution::Unresolved as i32,
        coarse_kind: CoarseEntityKind::ValueLike as i32,
        display_name: opaque_id.to_owned(),
        primary_definition: None,
        language_payload: None,
    }
}

fn string_set(raw: &Value, field: &str) -> Result<Vec<String>> {
    let values = raw
        .get(field)
        .and_then(Value::as_array)
        .with_context(|| format!("capability {field} must be an array"))?;
    let mut result = Vec::with_capacity(values.len());
    for value in values {
        let value = value
            .as_str()
            .with_context(|| format!("capability {field} item must be a string"))?;
        if result
            .last()
            .is_some_and(|prior: &String| prior.as_str() >= value)
        {
            bail!("capability {field} must be strictly sorted and unique");
        }
        result.push(value.to_owned());
    }
    Ok(result)
}

fn artifact_origin(value: &str) -> Result<ArtifactOrigin> {
    Ok(match value {
        "USER" => ArtifactOrigin::User,
        "GENERATED" => ArtifactOrigin::Generated,
        "VENDORED" => ArtifactOrigin::Vendored,
        "EXTERNAL" => ArtifactOrigin::External,
        other => bail!("unknown artifact origin {other}"),
    })
}

fn entity_resolution(value: &str) -> Result<EntityResolution> {
    Ok(match value {
        "RESOLVED" => EntityResolution::Resolved,
        "UNRESOLVED" => EntityResolution::Unresolved,
        "AMBIGUOUS" => EntityResolution::Ambiguous,
        "SYNTHETIC" => EntityResolution::Synthetic,
        other => bail!("unknown entity resolution {other}"),
    })
}

fn coarse_kind(value: &str) -> Result<CoarseEntityKind> {
    Ok(match value {
        "MODULE" => CoarseEntityKind::Module,
        "NAMESPACE" => CoarseEntityKind::Namespace,
        "TYPE_LIKE" => CoarseEntityKind::TypeLike,
        "CALLABLE" => CoarseEntityKind::Callable,
        "VALUE_LIKE" => CoarseEntityKind::ValueLike,
        "FIELD_LIKE" => CoarseEntityKind::FieldLike,
        "MACRO_LIKE" => CoarseEntityKind::MacroLike,
        other => bail!("unknown coarse entity kind {other}"),
    })
}

fn occurrence_role(value: &str) -> Result<OccurrenceRole> {
    Ok(match value {
        "DEFINITION" => OccurrenceRole::Definition,
        "DECLARATION" => OccurrenceRole::Declaration,
        "REFERENCE" => OccurrenceRole::Reference,
        "CALL" => OccurrenceRole::Call,
        "READ" => OccurrenceRole::Read,
        "WRITE" => OccurrenceRole::Write,
        "IMPORT" => OccurrenceRole::Import,
        "EXPORT" => OccurrenceRole::Export,
        other => bail!("unknown occurrence role {other}"),
    })
}

fn occurrence_origin(value: &str) -> Result<OccurrenceOrigin> {
    Ok(match value {
        "SOURCE" => OccurrenceOrigin::Source,
        "GENERATED" => OccurrenceOrigin::Generated,
        "SYNTHETIC" => OccurrenceOrigin::Synthetic,
        "MACRO_DERIVED" => OccurrenceOrigin::MacroDerived,
        "SOURCE_MAP_DERIVED" => OccurrenceOrigin::SourceMapDerived,
        other => bail!("unknown occurrence origin {other}"),
    })
}

fn boundary_consequence(value: &str) -> Result<BoundaryConsequence> {
    Ok(match value {
        "LOCAL_ONLY" => BoundaryConsequence::LocalOnly,
        "ENUMERATION_INCOMPLETE" => BoundaryConsequence::EnumerationIncomplete,
        "PROOF_INVALID" => BoundaryConsequence::ProofInvalid,
        other => bail!("unknown boundary consequence {other}"),
    })
}

fn enumeration(value: &str) -> Result<Enumeration> {
    Ok(match value {
        "COMPLETE_IN_SCOPE" => Enumeration::CompleteInScope,
        "PARTIAL" => Enumeration::Partial,
        "UNKNOWN" => Enumeration::Unknown,
        other => bail!("unknown enumeration {other}"),
    })
}

fn approximation(value: &str) -> Result<Approximation> {
    Ok(match value {
        "EXACT" => Approximation::Exact,
        "SOUND_OVER" => Approximation::SoundOver,
        "SOUND_UNDER" => Approximation::SoundUnder,
        "HEURISTIC" => Approximation::Heuristic,
        "NOT_APPLICABLE" => Approximation::NotApplicable,
        other => bail!("unknown approximation {other}"),
    })
}

fn grade(value: &str) -> Result<EvidenceGrade> {
    Ok(match value {
        "NAVIGATION" => EvidenceGrade::Navigation,
        "COMPILER_RESOLVED" => EvidenceGrade::CompilerResolved,
        "COMPILER_CHECKED" => EvidenceGrade::CompilerChecked,
        "SOUND_STATIC_IN_SCOPE" => EvidenceGrade::SoundStaticInScope,
        "TRANSLATION_VALIDATED" => EvidenceGrade::TranslationValidated,
        "BOUNDED_FORMAL" => EvidenceGrade::BoundedFormal,
        "CONTRACT_CHECKED" => EvidenceGrade::ContractChecked,
        "STATICALLY_APPROXIMATED" => EvidenceGrade::StaticallyApproximated,
        "TESTED" => EvidenceGrade::Tested,
        "RUNTIME_OBSERVED" => EvidenceGrade::RuntimeObserved,
        other => bail!("unsupported positive evidence grade {other}"),
    })
}

fn truth(value: &str) -> Result<Truth> {
    Ok(match value {
        "TRUE" => Truth::True,
        "FALSE" => Truth::False,
        "UNKNOWN" => Truth::Unknown,
        other => bail!("unknown truth {other}"),
    })
}

fn obligation_status(value: &str) -> Result<ObligationStatus> {
    Ok(match value {
        "PENDING" => ObligationStatus::Pending,
        "SATISFIED" => ObligationStatus::Satisfied,
        "VIOLATED" => ObligationStatus::Violated,
        "UNKNOWN" => ObligationStatus::Unknown,
        "UNSUPPORTED" => ObligationStatus::Unsupported,
        other => bail!("unknown obligation status {other}"),
    })
}

fn impact_class(value: &str) -> Result<ImpactClass> {
    Ok(match value {
        "DEFINITE" => ImpactClass::Definite,
        "POSSIBLE" => ImpactClass::Possible,
        other => bail!("unknown impact class {other}"),
    })
}
