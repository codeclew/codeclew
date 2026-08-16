use evidence_core::protocol::*;
use evidence_core::{
    CapabilityQuery, ContractRegistry, DecisionPolicy, EvidenceBundle, FrozenCoreContract,
    ObligationClosureContract, PolicyError, Validate, evidence_merkle_root, seal_content_digest,
    sha256_digest, validate_bundle,
};
use std::collections::BTreeSet;

type SnapshotMutation = fn(&mut WorkspaceAnalysisSnapshot);

fn digest(label: &str) -> String {
    sha256_digest(label.as_bytes())
}

fn schema(uri: &str) -> SchemaRef {
    SchemaRef {
        uri: uri.to_owned(),
        major: 1,
        minor: 0,
        specification_digest: digest(uri),
    }
}

fn payload(label: &str) -> TypedPayload {
    let bytes = label.as_bytes().to_vec();
    TypedPayload {
        schema: Some(schema("codeclew:test-payload")),
        media_type: "text/plain".to_owned(),
        content_digest: sha256_digest(&bytes),
        canonical_bytes: bytes,
    }
}

fn scope() -> Scope {
    let mut value = Scope {
        scope_uri: "codeclew:scope/workspace".to_owned(),
        selector: Some(payload("all-owned-sources")),
        content_digest: String::new(),
    };
    seal_content_digest(&mut value).unwrap();
    value
}

fn boundary(consequence: BoundaryConsequence) -> Boundary {
    let mut value = Boundary {
        boundary_id: "boundary-1".to_owned(),
        kind_uri: "codeclew:boundary/generated-source".to_owned(),
        origin: "adapter".to_owned(),
        consequence: consequence as i32,
        details: Some(payload("not analyzed")),
        evidence: vec![],
        content_digest: String::new(),
    };
    seal_content_digest(&mut value).unwrap();
    value
}

fn complete_coverage() -> Coverage {
    let mut value = Coverage {
        scopes: vec![scope()],
        enumeration: Enumeration::CompleteInScope as i32,
        approximation: Approximation::Exact as i32,
        boundaries: vec![],
        assumptions: vec![],
        content_digest: String::new(),
    };
    seal_content_digest(&mut value).unwrap();
    value
}

fn partial_coverage() -> Coverage {
    let mut value = Coverage {
        scopes: vec![scope()],
        enumeration: Enumeration::Partial as i32,
        approximation: Approximation::SoundUnder as i32,
        boundaries: vec![boundary(BoundaryConsequence::EnumerationIncomplete)],
        assumptions: vec![],
        content_digest: String::new(),
    };
    seal_content_digest(&mut value).unwrap();
    value
}

fn adapter() -> AdapterIdentity {
    AdapterIdentity {
        adapter_id: "codeclew:adapter/test".to_owned(),
        version: "1.0.0".to_owned(),
        binary_digest: digest("adapter-binary"),
    }
}

fn operation() -> OperationRef {
    OperationRef {
        uri: "codeclew:operation/find-references".to_owned(),
        version: "1.0.0".to_owned(),
        specification_digest: digest("find-references-v1"),
    }
}

fn snapshot(flags: &[&str]) -> WorkspaceAnalysisSnapshot {
    let mut value = WorkspaceAnalysisSnapshot {
        schema: Some(schema("codeclew:workspace-analysis-snapshot")),
        snapshot_id: String::new(),
        repository_tree_digest: digest("tree"),
        vcs_revision: Some("0123456789abcdef".to_owned()),
        dirty: false,
        sources: vec![SourceArtifact {
            artifact_id: "source-1".to_owned(),
            normalized_path: "src/lib.rs".to_owned(),
            content_digest: digest("source bytes"),
            origin: ArtifactOrigin::User as i32,
            generator_id: String::new(),
            generated_from: vec![],
        }],
        build_system_uri: "codeclew:build/provider-a".to_owned(),
        build_model_digest: digest("build-model"),
        build_configuration_digest: digest("build-config"),
        dependency_graph_digest: digest("dependencies"),
        toolchain: Some(ToolIdentity {
            tool_uri: "codeclew:tool/compiler".to_owned(),
            version: "1.92.0".to_owned(),
            distribution_digest: digest("toolchain-distribution"),
            plugins: vec![],
            language_payload: None,
        }),
        targets: vec![BuildTarget {
            target_id: "lib".to_owned(),
            configuration_digest: digest("target-lib"),
            enabled_features: vec!["default".to_owned()],
            platform: "aarch64-apple-darwin".to_owned(),
            compiler_flags: flags.iter().map(|flag| (*flag).to_owned()).collect(),
            language_payload: None,
        }],
        relevant_environment: vec![KeyValue {
            key: "COMPILER_FLAGS".to_owned(),
            value: "".to_owned(),
        }],
        generated_sources_manifest_digest: digest("no-generated-sources"),
        adapter: Some(adapter()),
        metadata: Some(SnapshotMetadata {
            created_at: "2026-08-13T00:00:00Z".to_owned(),
        }),
    };
    value.seal_snapshot_id().unwrap();
    value
}

fn descriptor(snapshot: &WorkspaceAnalysisSnapshot, grade: EvidenceGrade) -> CapabilityDescriptor {
    let mut value = CapabilityDescriptor {
        schema: Some(schema("codeclew:capability-descriptor")),
        key: Some(CapabilityKey {
            language_id: "opaque-language-a".to_owned(),
            adapter: Some(adapter()),
            snapshot_id: snapshot.snapshot_id.clone(),
            toolchain_digest: snapshot
                .toolchain
                .as_ref()
                .unwrap()
                .distribution_digest
                .clone(),
            build_configuration_digest: snapshot.build_configuration_digest.clone(),
            target_digest: snapshot.targets[0].configuration_digest.clone(),
            operation: Some(operation()),
            grade: grade as i32,
        }),
        input_domain_uris: vec!["codeclew:domain/source".to_owned()],
        output_schema: Some(schema("codeclew:evidence-batch")),
        guaranteed_coverage: Some(complete_coverage()),
        required_capability_digests: vec![],
        known_boundary_kind_uris: vec!["codeclew:boundary/generated-source".to_owned()],
        assumptions: vec![],
        supported_contour: vec![scope()],
        unsupported_contour: vec![],
        cost_class_uri: "codeclew:cost/indexed-query".to_owned(),
        content_digest: String::new(),
    };
    seal_content_digest(&mut value).unwrap();
    value
}

fn decision(descriptor: &CapabilityDescriptor) -> CapabilityDecision {
    let mut value = CapabilityDecision {
        status: SupportStatus::Supported as i32,
        descriptor: Some(descriptor.clone()),
        refusal: None,
        content_digest: String::new(),
    };
    seal_content_digest(&mut value).unwrap();
    value
}

fn assertion(truth: Truth) -> RelationAssertion {
    RelationAssertion {
        relation: Some(operation()),
        operands: vec![Operand {
            name: "symbol".to_owned(),
            value: Some(operand::Value::Identity("test::symbol".to_owned())),
        }],
        truth: truth as i32,
        language_payload: None,
    }
}

fn fact(snapshot: &WorkspaceAnalysisSnapshot, descriptor: &CapabilityDescriptor) -> EvidenceFact {
    let mut value = EvidenceFact {
        schema: Some(schema("codeclew:evidence-fact")),
        fact_id: "fact-1".to_owned(),
        snapshot_id: snapshot.snapshot_id.clone(),
        capability_descriptor_digest: descriptor.content_digest.clone(),
        assertion: Some(assertion(Truth::True)),
        provenance: vec![EvidenceRef {
            kind_uri: "codeclew:evidence/compiler-output".to_owned(),
            content_digest: digest("compiler output"),
        }],
        coverage: Some(complete_coverage()),
        content_digest: String::new(),
    };
    seal_content_digest(&mut value).unwrap();
    value
}

fn batch(
    snapshot: &WorkspaceAnalysisSnapshot,
    descriptor: &CapabilityDescriptor,
    fact: &EvidenceFact,
) -> EvidenceBatch {
    let mut value = EvidenceBatch {
        schema: Some(schema("codeclew:evidence-batch")),
        snapshot_id: snapshot.snapshot_id.clone(),
        capability_descriptor_digest: descriptor.content_digest.clone(),
        entities: vec![],
        occurrences: vec![],
        facts: vec![fact.clone()],
        artifacts: vec![EvidenceRef {
            kind_uri: "codeclew:evidence/compiler-output".to_owned(),
            content_digest: digest("compiler output"),
        }],
        content_digest: String::new(),
    };
    seal_content_digest(&mut value).unwrap();
    value
}

fn graph(
    snapshot: &WorkspaceAnalysisSnapshot,
    descriptor: &CapabilityDescriptor,
    fact: &EvidenceFact,
) -> ObligationGraph {
    let intent_digest = digest("intent");
    let mut obligation = Obligation {
        obligation_id: "obligation-1".to_owned(),
        origin_intent_digest: intent_digest.clone(),
        required_operation: Some(operation()),
        scope: Some(scope()),
        precondition: None,
        postcondition: None,
        accepted_grades: vec![EvidenceGrade::CompilerChecked as i32],
        dependency_ids: vec![],
        mandatory: true,
        status: ObligationStatus::Satisfied as i32,
        evidence_fact_ids: vec![fact.fact_id.clone()],
        unknown_reason: String::new(),
        content_digest: String::new(),
    };
    seal_content_digest(&mut obligation).unwrap();
    let mut value = ObligationGraph {
        schema: Some(schema("codeclew:obligation-graph")),
        snapshot_id: snapshot.snapshot_id.clone(),
        intent_digest,
        closure_capability_digest: descriptor.content_digest.clone(),
        closure_specification_digest: digest("closure-spec"),
        obligations: vec![obligation],
        content_digest: String::new(),
    };
    seal_content_digest(&mut value).unwrap();
    value
}

fn valid_bundle() -> EvidenceBundle {
    let snapshot = snapshot(&["--cfg", "feature=one"]);
    let descriptor = descriptor(&snapshot, EvidenceGrade::CompilerChecked);
    let fact = fact(&snapshot, &descriptor);
    EvidenceBundle {
        snapshot: snapshot.clone(),
        capabilities: vec![decision(&descriptor)],
        batches: vec![batch(&snapshot, &descriptor, &fact)],
        obligation_graphs: vec![graph(&snapshot, &descriptor, &fact)],
        verification_receipts: vec![],
        impact_receipts: vec![],
    }
}

fn claim(grade: EvidenceGrade) -> ClaimSpec {
    let mut value = ClaimSpec {
        claim: Some(assertion(Truth::True)),
        accepted_grades: vec![grade as i32],
        required_enumeration: Enumeration::CompleteInScope as i32,
        accepted_approximations: vec![Approximation::Exact as i32],
        reject_proof_invalid_boundary: true,
        composition_rule_uri: "codeclew:composition/all".to_owned(),
        content_digest: String::new(),
    };
    seal_content_digest(&mut value).unwrap();
    value
}

#[test]
fn frozen_contract_has_separate_verified_digests() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let contract = FrozenCoreContract::verify(root).unwrap();
    assert_ne!(
        contract.digests.protocol_schema_digest,
        contract.digests.core_specification_digest
    );
    assert_ne!(
        contract.digests.core_specification_digest,
        contract.digests.conformance_specification_digest
    );
    assert_ne!(
        contract.digests.decision_core_digest,
        contract.digests.adapter_contract_digest
    );
    assert_eq!(
        contract
            .adapter_contract_files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        vec![
            "crates/evidence-adapters/src/bin/evidence.rs",
            "crates/evidence-adapters/src/core_bridge.rs",
            "crates/evidence-adapters/src/lib.rs",
            "schemas/adapter_output.schema.json",
        ]
    );
}

#[test]
fn canonical_valid_bundle_is_accepted() {
    validate_bundle(&valid_bundle()).unwrap();
}

#[test]
fn ordered_compiler_flags_are_part_of_snapshot_identity() {
    let first = snapshot(&["--cfg", "feature=one"]);
    let reordered = snapshot(&["feature=one", "--cfg"]);
    first.validate().unwrap();
    reordered.validate().unwrap();
    assert_ne!(first.snapshot_id, reordered.snapshot_id);
}

#[test]
fn every_analysis_input_rejects_a_stale_snapshot_id() {
    let cases: &[(&str, SnapshotMutation)] = &[
        ("compiler flags", |value| {
            value.targets[0].compiler_flags.push("--test".to_owned());
        }),
        ("features", |value| {
            value.targets[0].enabled_features.push("extra".to_owned());
        }),
        ("dependencies", |value| {
            value.dependency_graph_digest = digest("changed dependencies");
        }),
        ("generated sources", |value| {
            value.generated_sources_manifest_digest = digest("changed generated sources");
        }),
        ("toolchain", |value| {
            value.toolchain.as_mut().unwrap().distribution_digest = digest("changed toolchain");
        }),
        ("adapter", |value| {
            value.adapter.as_mut().unwrap().binary_digest = digest("changed adapter");
        }),
    ];
    for (label, mutate) in cases {
        let mut value = snapshot(&["--cfg", "feature=one"]);
        mutate(&mut value);
        let errors = value.validate().expect_err(label);
        assert!(
            errors
                .errors()
                .iter()
                .any(|error| error.path == "snapshotId" && error.code == "digest_mismatch"),
            "{label}: {errors:?}"
        );
    }
}

#[test]
fn unsorted_true_set_is_rejected_but_compiler_flags_are_not_sorted() {
    let mut value = snapshot(&["z", "a"]);
    value.targets[0].enabled_features = vec!["z".to_owned(), "a".to_owned()];
    value.seal_snapshot_id().unwrap();
    let errors = value.validate().unwrap_err();
    assert!(errors.errors().iter().any(|error| {
        error.code == "not_canonical_set" && error.path == "targets[0].enabledFeatures"
    }));
}

#[test]
fn uppercase_digest_is_rejected() {
    let mut value = snapshot(&["--cfg", "feature=one"]);
    value.repository_tree_digest = format!("sha256:{}", "A".repeat(64));
    assert!(
        value.validate().unwrap_err().errors().iter().any(|error| {
            error.path == "repositoryTreeDigest" && error.code == "invalid_digest"
        })
    );
}

#[test]
fn false_completeness_is_rejected() {
    let mut value = complete_coverage();
    value.boundaries = vec![boundary(BoundaryConsequence::ProofInvalid)];
    seal_content_digest(&mut value).unwrap();
    assert!(
        value
            .validate()
            .unwrap_err()
            .errors()
            .iter()
            .any(|error| error.code == "false_completeness")
    );
}

#[test]
fn incomplete_coverage_requires_an_explicit_boundary() {
    let mut value = partial_coverage();
    value.boundaries.clear();
    seal_content_digest(&mut value).unwrap();
    assert!(
        value
            .validate()
            .unwrap_err()
            .errors()
            .iter()
            .any(|error| error.code == "missing_boundary")
    );
}

#[test]
fn snapshot_substitution_is_rejected() {
    let mut bundle = valid_bundle();
    bundle.batches[0].snapshot_id = digest("other snapshot");
    seal_content_digest(&mut bundle.batches[0]).unwrap();
    let errors = validate_bundle(&bundle).unwrap_err();
    assert!(
        errors
            .errors()
            .iter()
            .any(|error| error.code == "snapshot_mismatch")
    );
}

#[test]
fn capability_inputs_cannot_be_laundered_through_a_valid_snapshot_id() {
    let mut bundle = valid_bundle();
    bundle.capabilities[0]
        .descriptor
        .as_mut()
        .unwrap()
        .key
        .as_mut()
        .unwrap()
        .toolchain_digest = digest("unrelated toolchain");
    let errors = validate_bundle(&bundle).unwrap_err();
    assert!(errors.errors().iter().any(|error| {
        error.path == "capabilities[0].descriptor.key.toolchainDigest"
            && error.code == "toolchain_mismatch"
    }));
}

#[test]
fn capability_cannot_certify_a_different_operation() {
    let mut bundle = valid_bundle();
    let fact = &mut bundle.batches[0].facts[0];
    fact.assertion.as_mut().unwrap().relation = Some(OperationRef {
        uri: "codeclew:operation/unrelated".to_owned(),
        version: "1.0.0".to_owned(),
        specification_digest: digest("unrelated-operation"),
    });
    seal_content_digest(fact).unwrap();
    seal_content_digest(&mut bundle.batches[0]).unwrap();
    assert!(
        validate_bundle(&bundle)
            .unwrap_err()
            .errors()
            .iter()
            .any(|error| {
                error.path == "batches[0].facts" && error.code == "operation_mismatch"
            })
    );
}

#[test]
fn source_range_is_bound_to_snapshot_artifact_digest() {
    let mut bundle = valid_bundle();
    bundle.batches[0].entities.push(EntityRef {
        adapter_namespace: "codeclew:adapter/test".to_owned(),
        opaque_id: "test::symbol".to_owned(),
        resolution: EntityResolution::Resolved as i32,
        coarse_kind: CoarseEntityKind::Callable as i32,
        display_name: "symbol".to_owned(),
        primary_definition: Some(SourceRange {
            artifact_id: "source-1".to_owned(),
            artifact_content_digest: digest("stale source bytes"),
            start_byte: 0,
            end_byte: 1,
        }),
        language_payload: None,
    });
    seal_content_digest(&mut bundle.batches[0]).unwrap();
    assert!(
        validate_bundle(&bundle)
            .unwrap_err()
            .errors()
            .iter()
            .any(|error| {
                error.path == "batches[0].entities[0].primaryDefinition.artifactContentDigest"
                    && error.code == "artifact_digest_mismatch"
            })
    );
}

#[test]
fn content_tampering_is_rejected() {
    let mut bundle = valid_bundle();
    bundle.batches[0].facts[0].fact_id = "tampered".to_owned();
    let errors = validate_bundle(&bundle).unwrap_err();
    assert!(
        errors
            .errors()
            .iter()
            .any(|error| error.code == "digest_mismatch")
    );
}

#[test]
fn forged_content_digest_is_rejected() {
    let mut bundle = valid_bundle();
    bundle.batches[0].facts[0].content_digest = digest("attacker supplied digest");
    let errors = validate_bundle(&bundle).unwrap_err();
    assert!(errors.errors().iter().any(|error| {
        error.path == "batches[0].facts[0].contentDigest" && error.code == "digest_mismatch"
    }));
}

#[test]
fn verification_receipt_merkle_root_is_recomputed() {
    let mut bundle = valid_bundle();
    let evidence = vec![EvidenceRef {
        kind_uri: "codeclew:evidence/compiler-output".to_owned(),
        content_digest: digest("compiler output"),
    }];
    let mut receipt = VerificationReceipt {
        schema: Some(schema("codeclew:verification-receipt")),
        receipt_id: "receipt-1".to_owned(),
        before_snapshot_id: bundle.snapshot.snapshot_id.clone(),
        after_snapshot_id: None,
        verifier: Some(adapter()),
        claim: Some(claim(EvidenceGrade::CompilerChecked)),
        result: ClaimResult::Satisfied as i32,
        obligation_graph_digest: bundle.obligation_graphs[0].content_digest.clone(),
        coverage: Some(complete_coverage()),
        assumptions: vec![],
        evidence: evidence.clone(),
        evidence_merkle_root: evidence_merkle_root(&evidence).unwrap(),
        content_digest: String::new(),
    };
    seal_content_digest(&mut receipt).unwrap();
    bundle.verification_receipts.push(receipt);
    validate_bundle(&bundle).unwrap();

    bundle.verification_receipts[0].evidence_merkle_root = digest("forged merkle root");
    seal_content_digest(&mut bundle.verification_receipts[0]).unwrap();
    assert!(
        validate_bundle(&bundle)
            .unwrap_err()
            .errors()
            .iter()
            .any(|error| {
                error.path == "verificationReceipts[0].evidenceMerkleRoot"
                    && error.code == "digest_mismatch"
            })
    );
}

#[test]
fn evidence_grades_are_exact_categories_not_an_order() {
    let bundle = valid_bundle();
    let descriptor = bundle.capabilities[0].descriptor.clone().unwrap();
    let fact = bundle.batches[0].facts[0].clone();
    let mut registry = ContractRegistry::new();
    registry.register(descriptor.clone()).unwrap();
    registry.validate_closure().unwrap();
    let policy = DecisionPolicy::new(&registry);

    assert_eq!(
        policy
            .evaluate_claim(
                &claim(EvidenceGrade::CompilerChecked),
                std::slice::from_ref(&fact),
                &complete_coverage(),
            )
            .unwrap(),
        ClaimResult::Satisfied
    );
    assert_eq!(
        policy
            .evaluate_claim(
                &claim(EvidenceGrade::CompilerResolved),
                std::slice::from_ref(&fact),
                &complete_coverage(),
            )
            .unwrap(),
        ClaimResult::Unknown
    );

    let query = CapabilityQuery::from(descriptor.key.clone().unwrap());
    assert!(registry.find_exact(&query).is_some());
    let mut other_grade = query.key;
    other_grade.grade = EvidenceGrade::Navigation as i32;
    assert!(
        registry
            .find_exact(&CapabilityQuery::from(other_grade))
            .is_none()
    );
}

#[test]
fn missing_capability_fails_closed_to_unknown() {
    let bundle = valid_bundle();
    let fact = &bundle.batches[0].facts[0];
    let registry = ContractRegistry::new();
    let policy = DecisionPolicy::new(&registry);
    assert_eq!(
        policy
            .evaluate_claim(
                &claim(EvidenceGrade::CompilerChecked),
                std::slice::from_ref(fact),
                &complete_coverage(),
            )
            .unwrap(),
        ClaimResult::Unknown
    );
}

#[test]
fn proof_invalid_boundary_fails_closed_to_unknown() {
    let mut coverage = partial_coverage();
    coverage.boundaries = vec![boundary(BoundaryConsequence::ProofInvalid)];
    seal_content_digest(&mut coverage).unwrap();
    let registry = ContractRegistry::new();
    let policy = DecisionPolicy::new(&registry);
    assert_eq!(
        policy.evaluate_impact_coverage(&coverage, &[]).unwrap(),
        ClaimResult::Unknown
    );
}

#[test]
fn obligation_cycles_are_rejected() {
    let bundle = valid_bundle();
    let mut graph = bundle.obligation_graphs[0].clone();
    let mut second = graph.obligations[0].clone();
    second.obligation_id = "obligation-2".to_owned();
    second.dependency_ids = vec!["obligation-1".to_owned()];
    seal_content_digest(&mut second).unwrap();
    graph.obligations[0].dependency_ids = vec!["obligation-2".to_owned()];
    seal_content_digest(&mut graph.obligations[0]).unwrap();
    graph.obligations.push(second);
    seal_content_digest(&mut graph).unwrap();
    assert!(
        graph
            .validate()
            .unwrap_err()
            .errors()
            .iter()
            .any(|error| error.code == "cycle")
    );
}

#[test]
fn missing_or_extra_obligation_fails_closed() {
    let bundle = valid_bundle();
    let graph = bundle.obligation_graphs[0].clone();
    let registry = ContractRegistry::new();
    let policy = DecisionPolicy::new(&registry);

    let expected_with_missing = ObligationClosureContract {
        capability_digest: graph.closure_capability_digest.clone(),
        specification_digest: graph.closure_specification_digest.clone(),
        obligation_ids: BTreeSet::from(["obligation-1".to_owned(), "obligation-2".to_owned()]),
    };
    assert!(
        matches!(
            policy.evaluate_obligation_graph_exact(&graph, &expected_with_missing),
            Err(PolicyError::ObligationClosureMismatch { missing, extra })
                if missing == ["obligation-2"] && extra.is_empty()
        ),
        "missing obligation must be rejected"
    );

    let mut graph_with_extra = graph;
    let mut extra = graph_with_extra.obligations[0].clone();
    extra.obligation_id = "obligation-2".to_owned();
    seal_content_digest(&mut extra).unwrap();
    graph_with_extra.obligations.push(extra);
    seal_content_digest(&mut graph_with_extra).unwrap();
    let expected_without_extra = ObligationClosureContract {
        capability_digest: graph_with_extra.closure_capability_digest.clone(),
        specification_digest: graph_with_extra.closure_specification_digest.clone(),
        obligation_ids: BTreeSet::from(["obligation-1".to_owned()]),
    };
    assert!(
        matches!(
            policy.evaluate_obligation_graph_exact(&graph_with_extra, &expected_without_extra),
            Err(PolicyError::ObligationClosureMismatch { missing, extra })
                if missing.is_empty() && extra == ["obligation-2"]
        ),
        "extra obligation must be rejected"
    );
}
