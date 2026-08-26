use clew::canonical;
use clew::cas::CasObject;
use clew::thread_callables::{
    CallableBudgets, CallableBuildInput, CallableCompilationAuthority, CallableFactSetRequest,
    CallableMemberAuthority, CallablePairBinding, CallableSelectedCompilation, CallableTaskBinding,
    GraphCoverage, KOTLIN_SEMANTIC_FACT_SCHEMA, QualifiedCallablePayload, RelationshipAuthority,
};
use clew::thread_flow::{
    FlowBudgets, FlowCertainty, FlowDirection, FlowRequest, FlowRootKind, FlowStatus,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::process::Command;

const ROOT: &str = "callable:sample/Service.save#jvm:()Ljava/lang/String;";
const WRITE: &str = "callable:sample/Service.write#jvm:()Ljava/lang/String;";
const AUDIT: &str = "callable:sample/Service.audit#jvm:()Ljava/lang/String;";
const CLIENT: &str = "callable:sample/Client.save#jvm:()Ljava/lang/String;";

#[test]
fn exact_root_cycle_is_bounded_deterministic_and_evidence_linked() {
    let facts = fixture();
    let request = request(&facts, ROOT, 4);
    let first = clew::thread_flow::build(request.clone(), &facts).unwrap();
    let second = clew::thread_flow::build(request, &facts).unwrap();

    assert_eq!(first, second);
    assert_eq!(first.slice.status, FlowStatus::Complete);
    assert_eq!(first.slice.certainty, FlowCertainty::Verified);
    assert_eq!(first.slice.counts.nodes, 3);
    assert_eq!(first.slice.counts.edges, 3);
    assert_eq!(first.slice.counts.boundaries, 0);
    assert!(first.slice.flow_id.starts_with("thread-flow:sha256:"));
    for support in first
        .slice
        .nodes
        .iter()
        .flat_map(|node| &node.support_refs)
        .chain(first.slice.edges.iter().flat_map(|edge| &edge.support_refs))
    {
        assert!(!support.fact_id.is_empty());
        assert!(support.source.is_some());
        assert_eq!(
            support.provenance.input_payload_ref,
            support.input_payload_ref
        );
    }
    clew::thread_flow::verify_prepared(&first, &facts).unwrap();
}

#[test]
fn depth_truncation_and_unknown_root_fail_closed() {
    let facts = fixture();
    let truncated = clew::thread_flow::build(request(&facts, ROOT, 1), &facts).unwrap();
    assert_eq!(truncated.slice.status, FlowStatus::Truncated);
    assert_eq!(truncated.slice.certainty, FlowCertainty::Unsure);
    assert!(
        truncated
            .slice
            .boundaries
            .iter()
            .any(|boundary| boundary.code == "FLOW_DEPTH_TRUNCATED")
    );
    assert!(
        truncated
            .slice
            .verification_obligations
            .contains(&"INCREASE_MAX_DEPTH_OR_SELECT_NARROWER_ROOT".into())
    );

    let error = clew::thread_flow::build(
        request(
            &facts,
            "callable:sample/Service.missing#jvm:()Ljava/lang/String;",
            4,
        ),
        &facts,
    )
    .unwrap_err();
    assert_eq!(error.code, clew::error::ErrorCode::InvalidInput);
}

#[test]
fn foreign_bindings_and_corrupt_fact_shards_are_rejected() {
    let facts = fixture();
    let mut foreign = request(&facts, ROOT, 4);
    foreign.thread_id = "thread:foreign".into();
    assert_eq!(
        clew::thread_flow::build(foreign, &facts).unwrap_err().code,
        clew::error::ErrorCode::InvalidInput
    );

    let mut corrupt = facts.clone();
    corrupt.fact_shards[0].bytes.push(b'\n');
    assert_eq!(
        clew::thread_flow::build(request(&corrupt, ROOT, 4), &corrupt)
            .unwrap_err()
            .code,
        clew::error::ErrorCode::StateCorrupt
    );
}

#[test]
fn cross_member_target_is_a_visible_boundary_before_pair_flow() {
    let left = member("left");
    let right = member("right");
    let left_compilation = compilation("left");
    let right_compilation = compilation("right");
    let facts = build_fact_set(
        vec![
            qualified(
                left,
                left_compilation,
                descriptor("sample/Service.save", "src/Service.kt", 0),
            ),
            qualified(
                right.clone(),
                right_compilation.clone(),
                descriptor("sample/Client.save", "src/Client.kt", 0),
            ),
            qualified(
                right,
                right_compilation,
                relation(CLIENT, ROOT, "src/Client.kt", 10),
            ),
        ],
        RelationshipAuthority::VerifiedSameSnapshotCompilationDependency,
    );
    let mut request = request(&facts, CLIENT, 4);
    request.member_alias = "right".into();
    let flow = clew::thread_flow::build(request, &facts).unwrap();
    assert_eq!(flow.slice.counts.nodes, 1);
    assert_eq!(flow.slice.counts.edges, 0);
    assert_eq!(flow.slice.certainty, FlowCertainty::Unsure);
    assert!(
        flow.slice
            .boundaries
            .iter()
            .any(|boundary| boundary.code == "CROSS_MEMBER_NOT_EXPANDED")
    );
}

#[test]
fn public_cli_is_exact_root_read_only_and_closed() {
    let binary = env!("CARGO_BIN_EXE_clew");
    let help = Command::new(binary)
        .args(["thread", "flow", "--help"])
        .output()
        .unwrap();
    assert!(help.status.success());
    let stdout = String::from_utf8(help.stdout).unwrap();
    for option in [
        "--fact-set",
        "--pair-id",
        "--member",
        "--root-kind",
        "--direction",
        "--max-depth",
    ] {
        assert!(stdout.contains(option), "missing flow option {option}");
    }

    let rejected_root = Command::new(binary)
        .args([
            "thread",
            "flow",
            "--thread",
            "thread:test",
            "--fact-set",
            "thread-callables:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "--pair-id",
            "pair-one",
            "--member",
            "left",
            "--root-kind",
            "token",
            "--root",
            "save",
            "--direction",
            "downstream",
        ])
        .output()
        .unwrap();
    assert!(!rejected_root.status.success());

    for args in [
        &["thread", "publish"][..],
        &["plan", "flow"][..],
        &["task-run", "flow"][..],
    ] {
        assert!(
            !Command::new(binary)
                .args(args)
                .output()
                .unwrap()
                .status
                .success()
        );
    }
}

fn fixture() -> clew::thread_callables::PreparedCallableFactSet {
    let left = member("left");
    let right = member("right");
    let left_compilation = compilation("left");
    let right_compilation = compilation("right");
    let payloads = vec![
        qualified(
            left.clone(),
            left_compilation.clone(),
            descriptor("sample/Service.save", "src/Service.kt", 0),
        ),
        qualified(
            left.clone(),
            left_compilation.clone(),
            descriptor("sample/Service.write", "src/Service.kt", 20),
        ),
        qualified(
            left.clone(),
            left_compilation.clone(),
            descriptor("sample/Service.audit", "src/Service.kt", 40),
        ),
        qualified(
            left.clone(),
            left_compilation.clone(),
            relation(ROOT, WRITE, "src/Service.kt", 10),
        ),
        qualified(
            left.clone(),
            left_compilation.clone(),
            relation(WRITE, AUDIT, "src/Service.kt", 30),
        ),
        qualified(
            left.clone(),
            left_compilation.clone(),
            relation(AUDIT, ROOT, "src/Service.kt", 50),
        ),
        qualified(
            right.clone(),
            right_compilation.clone(),
            descriptor("sample/Client.read", "src/Client.kt", 0),
        ),
    ];
    build_fact_set(payloads, RelationshipAuthority::DeclaredTopology)
}

fn build_fact_set(
    payloads: Vec<QualifiedCallablePayload>,
    relationship_authority: RelationshipAuthority,
) -> clew::thread_callables::PreparedCallableFactSet {
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
            pairs: vec![CallablePairBinding {
                pair_id: "pair-one".into(),
                provider_member: "left".into(),
                consumer_member: "right".into(),
                relationship_authority,
                dependency_evidence_ref: (relationship_authority
                    == RelationshipAuthority::VerifiedSameSnapshotCompilationDependency)
                    .then(|| {
                        object(
                            "codeclew-compilation-dependency-evidence/1.0",
                            "pair-dependency",
                        )
                    }),
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

fn request(
    facts: &clew::thread_callables::PreparedCallableFactSet,
    root: &str,
    max_depth: usize,
) -> FlowRequest {
    FlowRequest {
        thread_id: facts.authority.thread_id.clone(),
        thread_authority_digest: facts.authority.thread_authority_digest.clone(),
        fact_set_id: facts.projection.fact_set_id.clone(),
        fact_set_authority_digest: facts.authority.authority_digest.clone(),
        pair_id: "pair-one".into(),
        member_alias: "left".into(),
        root_kind: FlowRootKind::FullSymbol,
        root: root.into(),
        direction: FlowDirection::Downstream,
        budgets: FlowBudgets::frozen(max_depth).unwrap(),
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

fn descriptor(callable: &str, file: &str, start: u64) -> Value {
    json!({
        "schema":"declaration-descriptor/0.1",
        "file":file,
        "start":start,
        "end":start + 8,
        "symbolIdentity":format!("callable:{callable}#jvm:()Ljava/lang/String;"),
        "declarationKind":"FUNCTION",
        "ownerIdentity":format!("class:{}", callable.rsplit_once('.').unwrap().0),
        "containment":[format!("class:{}", callable.rsplit_once('.').unwrap().0)],
        "visibility":"public",
        "effectiveVisibility":"public",
        "exportBoundary":"PUBLIC_API",
        "modality":"FINAL",
        "resolution":"PROVEN",
        "provider":"K2_FIR",
        "module":":app",
        "sourceSet":"main",
        "sourceProvenance":"COMPILER_UTF16_RANGE_TO_UTF8_BYTES",
        "compilerAuthority":"fir-facts-extractor/0.6",
        "typeParameters":[],
        "compilerCallableId":callable,
        "isOverride":false,
        "returnType":"kotlin/String",
        "returnNullable":false,
        "parameterTypes":[],
    })
}

fn relation(owner: &str, target: &str, file: &str, start: u64) -> Value {
    json!({
        "schema":"declaration-relation/0.1",
        "file":file,
        "start":start,
        "end":start + 6,
        "kind":"CALLS",
        "owner":owner,
        "target":target,
        "resolution":"PROVEN",
        "provider":"K2_FIR",
        "cfgNodeIds":[],
        "sourceProvenance":"COMPILER_UTF16_RANGE_TO_UTF8_BYTES",
        "orderProvenance":"FIR_SOURCE_RANGE",
    })
}

fn qualified(
    member: CallableMemberAuthority,
    compilation: CallableCompilationAuthority,
    payload: Value,
) -> QualifiedCallablePayload {
    let schema = payload["schema"].as_str().unwrap();
    let category = match schema {
        "declaration-descriptor/0.1" => "descriptor",
        "declaration-relation/0.1" => "relation",
        _ => unreachable!(),
    };
    let bytes = canonical::bytes(&payload).unwrap();
    let hash = canonical::hash_bytes(&bytes);
    let source_ref = payload["file"].as_str().map(|file| {
        object(
            "codeclew-repository-source-content/1.0",
            &format!("{}:{file}", member.member_alias),
        )
    });
    QualifiedCallablePayload {
        member,
        compilation,
        fact_key: format!(
            "kotlin:{category}:{}",
            hash.strip_prefix("sha256:").unwrap()
        ),
        payload_ref: CasObject::for_bytes(KOTLIN_SEMANTIC_FACT_SCHEMA, &bytes).unwrap(),
        source_ref,
        payload,
    }
}

fn digest(label: &str) -> String {
    canonical::hash(&label).unwrap()
}

fn object(schema: &str, label: &str) -> CasObject {
    CasObject::for_bytes(schema, label.as_bytes()).unwrap()
}
