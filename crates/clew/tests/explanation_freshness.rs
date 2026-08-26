use clew::canonical;
use clew::cas::CasObject;
use clew::explanation::{
    CLAIM_INPUT_SCHEMA, ClaimInput, ClaimInputDocument, ClaimPredicate, PreparedExplanation,
};
use clew::explanation_freshness::{FreshnessSide, FreshnessStatus};
use clew::thread_callables::{
    CallableBudgets, CallableBuildInput, CallableCompilationAuthority, CallableFactSetRequest,
    CallableMemberAuthority, CallablePairBinding, CallableSelectedCompilation, CallableTaskBinding,
    GraphCoverage, KOTLIN_SEMANTIC_FACT_SCHEMA, PreparedCallableFactSet, QualifiedCallablePayload,
    RelationshipAuthority,
};
use clew::thread_change_set::MemberCorrespondence;
use clew::thread_flow::{FlowBudgets, FlowDirection, FlowRequest, FlowRootKind, PreparedFlowSlice};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::process::Command;

const ROOT: &str = "callable:sample/Product.save#jvm:()Ljava/lang/String;";
const WRITE: &str = "callable:sample/Product.write#jvm:()Ljava/lang/String;";
const AUDIT: &str = "callable:sample/Product.audit#jvm:()Ljava/lang/String;";
const RENAMED: &str = "callable:sample/Product.persist#jvm:()Ljava/lang/String;";

#[test]
fn offsets_are_current_relation_changes_are_selective_and_shape_changes_are_stale() {
    let old = snapshot("thread:old", ROOT, WRITE, 0, false);
    let explanation = explanation(&old.flow);
    let shifted = snapshot("thread:shifted", ROOT, WRITE, 200, false);
    let current = compare(&old, &explanation, &shifted).unwrap();
    assert_eq!(current.report.status, FreshnessStatus::Current);
    assert!(current.report.affected_claims.is_empty());

    let changed_relation = snapshot("thread:relation", ROOT, AUDIT, 0, false);
    let partial = compare(&old, &explanation, &changed_relation).unwrap();
    assert_eq!(partial.report.status, FreshnessStatus::PartiallyStale);
    assert_eq!(partial.report.affected_claims.len(), 1);
    assert_eq!(partial.report.unaffected_claim_ids.len(), 1);
    assert_eq!(
        partial.report.affected_claims[0].regeneration_obligation,
        "REGENERATE_CLAIM_FROM_AGAINST_FLOW"
    );

    let changed_shape = snapshot("thread:shape", ROOT, WRITE, 0, true);
    let stale = compare(&old, &explanation, &changed_shape).unwrap();
    assert_eq!(stale.report.status, FreshnessStatus::Stale);
    assert_eq!(stale.report.affected_claims.len(), 2);
}

#[test]
fn renamed_root_is_unresolved_and_never_current() {
    let old = snapshot("thread:old", ROOT, WRITE, 0, false);
    let explanation = explanation(&old.flow);
    let renamed = snapshot("thread:renamed", RENAMED, WRITE, 0, false);
    let report = compare(&old, &explanation, &renamed).unwrap();
    assert_eq!(report.report.status, FreshnessStatus::Unresolved);
    assert_eq!(report.report.affected_claims.len(), 2);
}

#[test]
fn truncated_against_flow_is_unresolved() {
    let old = snapshot("thread:old", ROOT, WRITE, 0, false);
    let explanation = explanation(&old.flow);
    let truncated = snapshot_config("thread:truncated", ROOT, WRITE, 0, false, true, false, 1);
    assert_eq!(
        truncated.flow.slice.status,
        clew::thread_flow::FlowStatus::Truncated
    );
    let report = compare(&old, &explanation, &truncated).unwrap();
    assert_eq!(report.report.status, FreshnessStatus::Unresolved);
}

#[test]
fn new_relevant_boundary_invalidates_claims_without_rewriting_the_bundle() {
    let old = snapshot("thread:old", ROOT, WRITE, 0, false);
    let explanation = explanation(&old.flow);
    let old_bytes = explanation.bundle_bytes.clone();
    let boundary = snapshot_config("thread:boundary", ROOT, WRITE, 0, false, false, true, 4);
    let report = compare(&old, &explanation, &boundary).unwrap();
    assert_ne!(report.report.status, FreshnessStatus::Current);
    assert!(report.report.affected_claims.iter().any(|claim| {
        claim
            .reasons
            .contains(&clew::explanation_freshness::FreshnessReason::NewRelevantBoundary)
    }));
    assert_eq!(explanation.bundle_bytes, old_bytes);
}

#[test]
fn repository_correspondence_and_old_cas_corruption_fail_closed() {
    let old = snapshot("thread:old", ROOT, WRITE, 0, false);
    let explanation = explanation(&old.flow);
    let new = snapshot("thread:new", ROOT, WRITE, 0, false);
    let mut wrong = correspondence();
    wrong[0].after_member_alias = "right".into();
    assert_eq!(
        clew::explanation_freshness::build(side(&old), &explanation, side(&new), wrong,)
            .unwrap_err()
            .code,
        clew::error::ErrorCode::InvalidInput
    );

    let mut corrupt = old;
    corrupt.facts.fact_shards[0].bytes.push(b'\n');
    assert_eq!(
        compare(&corrupt, &explanation, &new).unwrap_err().code,
        clew::error::ErrorCode::StateCorrupt
    );
}

#[test]
fn report_is_deterministic_bounded_and_rebuild_verifiable() {
    let old = snapshot("thread:old", ROOT, WRITE, 0, false);
    let explanation = explanation(&old.flow);
    let new = snapshot("thread:new", ROOT, AUDIT, 0, false);
    let first = compare(&old, &explanation, &new).unwrap();
    let second = compare(&old, &explanation, &new).unwrap();
    assert_eq!(first, second);
    assert!(
        first
            .report
            .freshness_id
            .starts_with("explanation-freshness:sha256:")
    );
    assert!(canonical::bytes(&first.projection).unwrap().len() < 64 * 1024);
    for required in [
        &explanation.bundle_ref,
        &old.flow.slice_ref,
        &new.flow.slice_ref,
    ] {
        assert!(first.report.retained_closure.contains(required));
    }
    clew::explanation_freshness::verify_prepared(side(&old), &explanation, side(&new), &first)
        .unwrap();
}

#[test]
fn explanation_status_cli_requires_both_binding_chains_and_correspondence() {
    let binary = env!("CARGO_BIN_EXE_clew");
    let help = Command::new(binary)
        .args(["thread", "explanation-status", "--help"])
        .output()
        .unwrap();
    assert!(help.status.success());
    let text = String::from_utf8(help.stdout).unwrap();
    for option in [
        "--thread",
        "--explanation",
        "--against-thread",
        "--against-fact-set",
        "--against-flow",
        "--member-correspondence",
    ] {
        assert!(text.contains(option), "missing CLI option {option}");
    }
}

struct Snapshot {
    thread_id: String,
    thread_digest: String,
    facts: PreparedCallableFactSet,
    flow: PreparedFlowSlice,
}

fn snapshot(
    thread_id: &str,
    root: &str,
    relation_target: &str,
    offset: u64,
    nullable: bool,
) -> Snapshot {
    snapshot_config(
        thread_id,
        root,
        relation_target,
        offset,
        nullable,
        false,
        false,
        4,
    )
}

#[allow(clippy::too_many_arguments)]
fn snapshot_config(
    thread_id: &str,
    root: &str,
    relation_target: &str,
    offset: u64,
    nullable: bool,
    extra_relation: bool,
    relation_boundary: bool,
    max_depth: usize,
) -> Snapshot {
    let mut payloads = vec![
        qualified(
            thread_id,
            "left",
            descriptor(root, "src/Product.kt", offset, nullable),
        ),
        qualified(
            thread_id,
            "left",
            descriptor(WRITE, "src/Product.kt", offset + 20, false),
        ),
        qualified(
            thread_id,
            "left",
            descriptor(AUDIT, "src/Product.kt", offset + 40, false),
        ),
        qualified(
            thread_id,
            "left",
            relation(
                callable_id(root),
                relation_target,
                "src/Product.kt",
                offset + 10,
            ),
        ),
        qualified(
            thread_id,
            "right",
            descriptor(
                "callable:sample/Client.read#jvm:()Ljava/lang/String;",
                "src/Client.kt",
                offset,
                false,
            ),
        ),
    ];
    if extra_relation {
        payloads.push(qualified(
            thread_id,
            "left",
            relation(
                callable_id(relation_target),
                AUDIT,
                "src/Product.kt",
                offset + 30,
            ),
        ));
    }
    if relation_boundary {
        payloads.push(qualified(
            thread_id,
            "left",
            boundary(ROOT, "src/Product.kt", offset + 12),
        ));
    }
    let facts = build_fact_set(thread_id, payloads);
    let flow = clew::thread_flow::build(
        FlowRequest {
            thread_id: thread_id.into(),
            thread_authority_digest: digest(&format!("authority-{thread_id}")),
            fact_set_id: facts.projection.fact_set_id.clone(),
            fact_set_authority_digest: facts.authority.authority_digest.clone(),
            pair_id: "pair-one".into(),
            member_alias: "left".into(),
            root_kind: FlowRootKind::FullSymbol,
            root: root.into(),
            direction: FlowDirection::Downstream,
            budgets: FlowBudgets::frozen(max_depth).unwrap(),
        },
        &facts,
    )
    .unwrap();
    Snapshot {
        thread_id: thread_id.into(),
        thread_digest: digest(&format!("authority-{thread_id}")),
        facts,
        flow,
    }
}

fn explanation(flow: &PreparedFlowSlice) -> PreparedExplanation {
    let root_node = flow
        .slice
        .nodes
        .iter()
        .find(|node| node.symbol_identity == flow.slice.request.root)
        .unwrap();
    let edge = &flow.slice.edges[0];
    let target = flow
        .slice
        .nodes
        .iter()
        .find(|node| node.node_id == edge.target_node_id)
        .unwrap();
    clew::explanation::build(
        &flow.slice,
        &flow.slice_ref,
        ClaimInputDocument {
            schema: CLAIM_INPUT_SCHEMA.into(),
            flow_id: flow.slice.flow_id.clone(),
            claims: vec![
                ClaimInput {
                    local_id: "call".into(),
                    locale: "en".into(),
                    text: "Save calls its persistence dependency.".into(),
                    predicate: ClaimPredicate::CallExists {
                        subject: root_node.symbol_identity.clone(),
                        object: target.symbol_identity.clone(),
                    },
                    support_refs: vec![edge.edge_id.clone()],
                    boundary_refs: vec![],
                },
                ClaimInput {
                    local_id: "summary".into(),
                    locale: "en".into(),
                    text: "The product save operation starts here.".into(),
                    predicate: ClaimPredicate::NarrativeSummary {
                        subject: root_node.symbol_identity.clone(),
                    },
                    support_refs: vec![root_node.node_id.clone()],
                    boundary_refs: vec![],
                },
            ],
        },
    )
    .unwrap()
}

fn compare(
    old: &Snapshot,
    explanation: &PreparedExplanation,
    against: &Snapshot,
) -> Result<clew::explanation_freshness::PreparedExplanationFreshness, clew::error::ClewError> {
    clew::explanation_freshness::build(side(old), explanation, side(against), correspondence())
}

fn side(snapshot: &Snapshot) -> FreshnessSide<'_> {
    FreshnessSide {
        thread_id: &snapshot.thread_id,
        thread_authority_digest: &snapshot.thread_digest,
        fact_set: &snapshot.facts,
        flow: &snapshot.flow,
    }
}

fn correspondence() -> Vec<MemberCorrespondence> {
    vec![
        MemberCorrespondence {
            before_member_alias: "left".into(),
            after_member_alias: "left".into(),
        },
        MemberCorrespondence {
            before_member_alias: "right".into(),
            after_member_alias: "right".into(),
        },
    ]
}

fn build_fact_set(
    thread_id: &str,
    payloads: Vec<QualifiedCallablePayload>,
) -> PreparedCallableFactSet {
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
            thread_id: thread_id.into(),
            thread_authority_digest: digest(&format!("authority-{thread_id}")),
            thread_context_id: format!("thread-context:{thread_id}"),
            thread_context_authority_digest: digest(&format!("context-{thread_id}")),
            profile_digest: digest("stable-profile"),
            tasks: vec![CallableTaskBinding {
                task_id: "task-one".into(),
                pair_id: "pair-one".into(),
                terms: vec!["save".into()],
            }],
            pairs: vec![CallablePairBinding {
                pair_id: "pair-one".into(),
                provider_member: "left".into(),
                consumer_member: "right".into(),
                relationship_authority:
                    RelationshipAuthority::VerifiedSameSnapshotCompilationDependency,
                dependency_evidence_ref: Some(object(
                    "codeclew-compilation-dependency-evidence/1.0",
                    "pair-one",
                )),
            }],
            budgets: CallableBudgets::frozen(),
        },
        CallableBuildInput {
            visited_fact_count: payloads.len(),
            visited_payload_bytes,
            selected_compilations,
            payloads,
        },
    )
    .unwrap()
}

fn qualified(thread_id: &str, alias: &str, payload: Value) -> QualifiedCallablePayload {
    let bytes = canonical::bytes(&payload).unwrap();
    let schema = payload["schema"].as_str().unwrap();
    let category = match schema {
        "declaration-descriptor/0.1" => "descriptor",
        "declaration-relation/0.1" => "relation",
        "declaration-relation-boundary/0.1" => "relation-boundary",
        _ => unreachable!(),
    };
    QualifiedCallablePayload {
        member: member(thread_id, alias),
        compilation: compilation(thread_id, alias),
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
                &format!("{thread_id}:{alias}:{file}"),
            )
        }),
        payload,
    }
}

fn member(thread_id: &str, alias: &str) -> CallableMemberAuthority {
    CallableMemberAuthority {
        member_alias: alias.into(),
        service_alias: format!("{alias}-service"),
        session_id: format!("session:{thread_id}:{alias}"),
        session_authority_digest: digest(&format!("session-{thread_id}-{alias}")),
        repository_key: format!("repository-{alias}"),
        base_revision: digest(&format!("revision-{thread_id}-{alias}")),
        snapshot_ref: object(
            "codeclew-repository-input-snapshot/1.0",
            &format!("snapshot-{thread_id}-{alias}"),
        ),
    }
}

fn compilation(thread_id: &str, alias: &str) -> CallableCompilationAuthority {
    CallableCompilationAuthority {
        compilation_id: ":app/main".into(),
        generation_id: digest(&format!("generation-id-{thread_id}-{alias}")),
        generation_ref: object(
            "codeclew-generation-manifest/2.0",
            &format!("generation-{thread_id}-{alias}"),
        ),
        semantic_authority: "K2_FIR".into(),
        extractor_id: "fir-facts-extractor/0.6".into(),
        adapter_digest: digest("adapter"),
        runtime_digest: digest("runtime"),
        descriptor_coverage: GraphCoverage::CompleteSupportedSubset,
        relation_coverage: if thread_id == "thread:boundary" {
            GraphCoverage::Partial
        } else {
            GraphCoverage::CompleteSupportedSubset
        },
    }
}

fn descriptor(symbol: &str, file: &str, start: u64, nullable: bool) -> Value {
    let callable = callable_id(symbol);
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
        "returnNullable":nullable, "parameterTypes":[]
    })
}

fn relation(owner: &str, target: &str, file: &str, start: u64) -> Value {
    json!({
        "schema":"declaration-relation/0.1", "file":file, "start":start, "end":start + 6,
        "kind":"CALLS", "owner":owner, "target":target, "resolution":"PROVEN",
        "provider":"K2_FIR", "cfgNodeIds":[],
        "orderKey":start,
        "sourceProvenance":"COMPILER_UTF16_RANGE_TO_UTF8_BYTES",
        "orderProvenance":"FIR_SOURCE_RANGE"
    })
}

fn boundary(subject: &str, file: &str, start: u64) -> Value {
    json!({
        "schema":"declaration-relation-boundary/0.1", "file":file,
        "start":start, "end":start + 1, "owner":subject,
        "stage":"CALL_RESOLUTION",
        "code":"UNRESOLVED_RELATION_TARGET", "resolution":"UNKNOWN",
        "provider":"K2_FIR"
    })
}

fn callable_id(symbol: &str) -> &str {
    symbol
        .strip_prefix("callable:")
        .unwrap()
        .split("#jvm:")
        .next()
        .unwrap()
}

fn digest(label: &str) -> String {
    canonical::hash(&label).unwrap()
}

fn object(schema: &str, label: &str) -> CasObject {
    CasObject::for_bytes(schema, label.as_bytes()).unwrap()
}
