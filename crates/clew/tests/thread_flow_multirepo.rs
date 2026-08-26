use clew::canonical;
use clew::cas::CasObject;
use clew::explanation::{
    CLAIM_INPUT_SCHEMA, ClaimAuthority, ClaimInput, ClaimInputDocument, ClaimPredicate,
};
use clew::thread_callables::{
    CallableBudgets, CallableBuildInput, CallableCompilationAuthority, CallableFactSetRequest,
    CallableMemberAuthority, CallablePairBinding, CallableSelectedCompilation, CallableTaskBinding,
    GraphCoverage, KOTLIN_SEMANTIC_FACT_SCHEMA, QualifiedCallablePayload, RelationshipAuthority,
};
use clew::thread_flow::{FlowBudgets, FlowDirection, FlowNodeKind, FlowRequest, FlowRootKind};
use serde_json::{Value, json};
use std::collections::BTreeMap;

const PROVIDER: &str = "callable:sample/Service.save#jvm:()Ljava/lang/String;";
const CONSUMER: &str = "callable:sample/Client.save#jvm:()Ljava/lang/String;";
const THIRD: &str = "callable:sample/Audit.record#jvm:()Ljava/lang/String;";

#[test]
fn declared_pair_handoff_is_qualified_deterministic_and_never_compiler_proven() {
    let facts = fact_set(
        vec![pair(
            "pair-one",
            "left",
            "right",
            RelationshipAuthority::DeclaredTopology,
        )],
        vec![
            qualified("left", descriptor(PROVIDER, "src/Service.kt", 0)),
            qualified("right", descriptor(CONSUMER, "src/Client.kt", 0)),
            qualified(
                "right",
                relation("sample/Client.save", PROVIDER, "src/Client.kt", 20),
            ),
        ],
    )
    .unwrap();
    let request = request(&facts, "pair-one", "right", CONSUMER);
    let first = clew::thread_flow::build(request.clone(), &facts).unwrap();
    let second = clew::thread_flow::build(request, &facts).unwrap();
    assert_eq!(first, second);
    assert_eq!(first.slice.nodes.len(), 2);
    assert_eq!(first.slice.edges.len(), 1);

    let edge = &first.slice.edges[0];
    assert_eq!(edge.source_member_alias, "right");
    assert_eq!(edge.source_service_alias, "right-service");
    assert_eq!(edge.target_member_alias, "left");
    assert_eq!(edge.target_service_alias, "left-service");
    assert_ne!(
        edge.source_repository_namespace,
        edge.target_repository_namespace
    );
    assert_eq!(
        edge.relationship_authority,
        RelationshipAuthority::DeclaredTopology
    );
    assert!(edge.cfg_graph_id.is_none());
    let target = first
        .slice
        .nodes
        .iter()
        .find(|node| node.node_id == edge.target_node_id)
        .unwrap();
    assert_eq!(target.node_kind, FlowNodeKind::Boundary);
    assert_eq!(target.member_alias, "left");
    let boundary = first
        .slice
        .boundaries
        .iter()
        .find(|boundary| boundary.code == "DECLARED_TOPOLOGY_HANDOFF")
        .unwrap();
    assert!(
        first
            .slice
            .verification_obligations
            .contains(&"VERIFY_RUNTIME_COMPONENT_HANDOFF".into())
    );

    let explanation = clew::explanation::build(
        &first.slice,
        &first.slice_ref,
        ClaimInputDocument {
            schema: CLAIM_INPUT_SCHEMA.into(),
            flow_id: first.slice.flow_id.clone(),
            claims: vec![ClaimInput {
                local_id: "handoff".into(),
                locale: "en".into(),
                text: "The consumer has a declared topology handoff to the provider.".into(),
                predicate: ClaimPredicate::ComponentHandoff {
                    subject: "right-service".into(),
                    object: "left-service".into(),
                },
                support_refs: vec![edge.edge_id.clone()],
                boundary_refs: vec![boundary.boundary_id.clone()],
            }],
        },
    )
    .unwrap();
    assert_eq!(
        explanation.bundle.claims[0].authority,
        ClaimAuthority::Declared
    );
}

#[test]
fn unbound_and_exact_dependency_promotion_fail_closed() {
    for (authority, expected) in [
        (RelationshipAuthority::Unbound, "UNBOUND_COMPONENT_TARGET"),
        (
            RelationshipAuthority::VerifiedSameSnapshotCompilationDependency,
            "UNSUPPORTED_EXACT_PAIR_DEPENDENCY",
        ),
    ] {
        let facts = fact_set(
            vec![pair("pair-one", "left", "right", authority)],
            vec![
                qualified("left", descriptor(PROVIDER, "src/Service.kt", 0)),
                qualified("right", descriptor(CONSUMER, "src/Client.kt", 0)),
                qualified(
                    "right",
                    relation("sample/Client.save", PROVIDER, "src/Client.kt", 20),
                ),
            ],
        )
        .unwrap();
        let flow = clew::thread_flow::build(request(&facts, "pair-one", "right", CONSUMER), &facts)
            .unwrap();
        assert_eq!(flow.slice.nodes.len(), 1);
        assert!(flow.slice.edges.is_empty());
        assert!(
            flow.slice
                .boundaries
                .iter()
                .any(|boundary| boundary.code == expected)
        );
    }
}

#[test]
fn selected_pair_never_silently_walks_into_a_third_member() {
    let facts = fact_set(
        vec![
            pair(
                "pair-one",
                "left",
                "right",
                RelationshipAuthority::DeclaredTopology,
            ),
            pair(
                "pair-two",
                "third",
                "right",
                RelationshipAuthority::VerifiedSameSnapshotCompilationDependency,
            ),
        ],
        vec![
            qualified("left", descriptor(PROVIDER, "src/Service.kt", 0)),
            qualified("right", descriptor(CONSUMER, "src/Client.kt", 0)),
            qualified("third", descriptor(THIRD, "src/Audit.kt", 0)),
            qualified(
                "right",
                relation("sample/Client.save", THIRD, "src/Client.kt", 20),
            ),
        ],
    )
    .unwrap();
    let flow =
        clew::thread_flow::build(request(&facts, "pair-one", "right", CONSUMER), &facts).unwrap();
    assert!(flow.slice.edges.is_empty());
    assert!(
        flow.slice
            .nodes
            .iter()
            .all(|node| node.member_alias != "third")
    );
    assert!(
        flow.slice
            .boundaries
            .iter()
            .any(|boundary| boundary.code == "TARGET_OUTSIDE_SELECTED_PAIR")
    );
}

#[test]
fn one_unresolved_relation_is_not_attributed_across_multiple_declared_pairs() {
    let facts = fact_set(
        vec![
            pair(
                "pair-one",
                "left",
                "right",
                RelationshipAuthority::DeclaredTopology,
            ),
            pair(
                "pair-two",
                "third",
                "right",
                RelationshipAuthority::DeclaredTopology,
            ),
        ],
        vec![
            qualified("left", descriptor(PROVIDER, "src/Service.kt", 0)),
            qualified("right", descriptor(CONSUMER, "src/Client.kt", 0)),
            qualified("third", descriptor(THIRD, "src/Audit.kt", 0)),
            qualified(
                "right",
                relation("sample/Client.save", PROVIDER, "src/Client.kt", 20),
            ),
        ],
    )
    .unwrap();
    let flow =
        clew::thread_flow::build(request(&facts, "pair-one", "right", CONSUMER), &facts).unwrap();
    assert!(flow.slice.edges.is_empty());
    assert!(
        flow.slice
            .boundaries
            .iter()
            .any(|boundary| boundary.code == "AMBIGUOUS_DECLARED_TOPOLOGY_HANDOFF")
    );
}

#[test]
fn identical_callable_ids_in_two_namespaces_remain_member_qualified() {
    let facts = fact_set(
        vec![pair(
            "pair-one",
            "left",
            "right",
            RelationshipAuthority::DeclaredTopology,
        )],
        vec![
            qualified("left", descriptor(PROVIDER, "src/Service.kt", 0)),
            qualified("right", descriptor(PROVIDER, "src/LocalService.kt", 0)),
            qualified("right", descriptor(CONSUMER, "src/Client.kt", 0)),
            qualified(
                "right",
                relation("sample/Client.save", PROVIDER, "src/Client.kt", 20),
            ),
        ],
    )
    .unwrap();
    let flow =
        clew::thread_flow::build(request(&facts, "pair-one", "right", CONSUMER), &facts).unwrap();
    assert_eq!(flow.slice.edges.len(), 1);
    assert!(
        flow.slice
            .nodes
            .iter()
            .all(|node| node.member_alias == "right")
    );
    assert_eq!(flow.slice.edges[0].source_member_alias, "right");
    assert_eq!(flow.slice.edges[0].target_member_alias, "right");
}

#[test]
fn pair_with_missing_consumer_is_rejected_before_flow_construction() {
    let error = fact_set(
        vec![pair(
            "pair-one",
            "left",
            "missing",
            RelationshipAuthority::DeclaredTopology,
        )],
        vec![
            qualified("left", descriptor(PROVIDER, "src/Service.kt", 0)),
            qualified("third", descriptor(THIRD, "src/Audit.kt", 0)),
        ],
    )
    .unwrap_err();
    assert_eq!(error.code, clew::error::ErrorCode::InvalidInput);
}

fn fact_set(
    pairs: Vec<CallablePairBinding>,
    payloads: Vec<QualifiedCallablePayload>,
) -> Result<clew::thread_callables::PreparedCallableFactSet, clew::error::ClewError> {
    let visited_payload_bytes = payloads
        .iter()
        .map(|payload| canonical::bytes(&payload.payload).unwrap().len())
        .sum();
    let selected_compilations = payloads
        .iter()
        .map(|payload| {
            (
                (
                    payload.member.member_alias.clone(),
                    payload.compilation.compilation_id.clone(),
                ),
                CallableSelectedCompilation {
                    member: payload.member.clone(),
                    compilation: payload.compilation.clone(),
                },
            )
        })
        .collect::<BTreeMap<_, _>>()
        .into_values()
        .collect();
    clew::thread_callables::build(
        CallableFactSetRequest {
            thread_id: "thread:test".into(),
            thread_authority_digest: digest("thread"),
            thread_context_id: "thread-context:test".into(),
            thread_context_authority_digest: digest("context"),
            profile_digest: digest("profile"),
            tasks: vec![CallableTaskBinding {
                task_id: "task-one".into(),
                pair_id: "pair-one".into(),
                terms: vec!["save".into()],
            }],
            pairs,
            budgets: CallableBudgets::frozen(),
        },
        CallableBuildInput {
            visited_fact_count: payloads.len(),
            visited_payload_bytes,
            selected_compilations,
            payloads,
        },
    )
}

fn pair(
    pair_id: &str,
    provider: &str,
    consumer: &str,
    authority: RelationshipAuthority,
) -> CallablePairBinding {
    CallablePairBinding {
        pair_id: pair_id.into(),
        provider_member: provider.into(),
        consumer_member: consumer.into(),
        relationship_authority: authority,
        dependency_evidence_ref: (authority
            == RelationshipAuthority::VerifiedSameSnapshotCompilationDependency)
            .then(|| object("codeclew-compilation-dependency-evidence/1.0", pair_id)),
    }
}

fn request(
    facts: &clew::thread_callables::PreparedCallableFactSet,
    pair_id: &str,
    member_alias: &str,
    root: &str,
) -> FlowRequest {
    FlowRequest {
        thread_id: facts.authority.thread_id.clone(),
        thread_authority_digest: facts.authority.thread_authority_digest.clone(),
        fact_set_id: facts.projection.fact_set_id.clone(),
        fact_set_authority_digest: facts.authority.authority_digest.clone(),
        pair_id: pair_id.into(),
        member_alias: member_alias.into(),
        root_kind: FlowRootKind::FullSymbol,
        root: root.into(),
        direction: FlowDirection::Downstream,
        budgets: FlowBudgets::frozen(8).unwrap(),
    }
}

fn qualified(alias: &str, payload: Value) -> QualifiedCallablePayload {
    let member = member(alias);
    let compilation = compilation(alias);
    let schema = payload["schema"].as_str().unwrap();
    let category = if schema == "declaration-descriptor/0.1" {
        "descriptor"
    } else {
        "relation"
    };
    let bytes = canonical::bytes(&payload).unwrap();
    QualifiedCallablePayload {
        member: member.clone(),
        compilation,
        fact_key: format!(
            "kotlin:{category}:{}",
            canonical::hash_bytes(&bytes)
                .strip_prefix("sha256:")
                .unwrap()
        ),
        payload_ref: CasObject::for_bytes(KOTLIN_SEMANTIC_FACT_SCHEMA, &bytes).unwrap(),
        source_ref: payload["file"].as_str().map(|file| {
            object(
                "codeclew-repository-source-content/1.0",
                &format!("{alias}:{file}"),
            )
        }),
        payload,
    }
}

fn member(alias: &str) -> CallableMemberAuthority {
    CallableMemberAuthority {
        member_alias: alias.into(),
        service_alias: format!("{alias}-service"),
        session_id: format!("session:{alias}"),
        session_authority_digest: digest(&format!("session-{alias}")),
        repository_key: format!("repository-{alias}"),
        base_revision: digest(&format!("revision-{alias}")),
        snapshot_ref: object(
            "codeclew-repository-input-snapshot/1.0",
            &format!("snapshot-{alias}"),
        ),
    }
}

fn compilation(alias: &str) -> CallableCompilationAuthority {
    CallableCompilationAuthority {
        compilation_id: ":app/main".into(),
        generation_id: digest(&format!("generation-id-{alias}")),
        generation_ref: object(
            "codeclew-generation-manifest/2.0",
            &format!("generation-{alias}"),
        ),
        semantic_authority: "K2_FIR".into(),
        extractor_id: "fir-facts-extractor/0.6".into(),
        adapter_digest: digest("adapter"),
        runtime_digest: digest("runtime"),
        descriptor_coverage: GraphCoverage::CompleteSupportedSubset,
        relation_coverage: GraphCoverage::CompleteSupportedSubset,
    }
}

fn descriptor(symbol: &str, file: &str, start: u64) -> Value {
    let callable = symbol
        .strip_prefix("callable:")
        .unwrap()
        .split("#jvm:")
        .next()
        .unwrap();
    json!({
        "schema":"declaration-descriptor/0.1", "file":file, "start":start, "end":start + 8,
        "symbolIdentity":symbol, "declarationKind":"FUNCTION",
        "ownerIdentity":format!("class:{}", callable.rsplit_once('.').unwrap().0),
        "containment":[format!("class:{}", callable.rsplit_once('.').unwrap().0)],
        "visibility":"public", "effectiveVisibility":"public", "exportBoundary":"PUBLIC_API",
        "modality":"FINAL", "resolution":"PROVEN", "provider":"K2_FIR", "module":":app",
        "sourceSet":"main", "sourceProvenance":"COMPILER_UTF16_RANGE_TO_UTF8_BYTES",
        "compilerAuthority":"fir-facts-extractor/0.6", "typeParameters":[],
        "compilerCallableId":callable, "isOverride":false, "returnType":"kotlin/String",
        "returnNullable":false, "parameterTypes":[]
    })
}

fn relation(owner: &str, target: &str, file: &str, start: u64) -> Value {
    json!({
        "schema":"declaration-relation/0.1", "file":file, "start":start, "end":start + 6,
        "kind":"CALLS", "owner":owner, "target":target, "resolution":"PROVEN",
        "provider":"K2_FIR", "cfgNodeIds":[],
        "sourceProvenance":"COMPILER_UTF16_RANGE_TO_UTF8_BYTES",
        "orderProvenance":"FIR_SOURCE_RANGE"
    })
}

fn digest(label: &str) -> String {
    canonical::hash(&label).unwrap()
}

fn object(schema: &str, label: &str) -> CasObject {
    CasObject::for_bytes(schema, label.as_bytes()).unwrap()
}
