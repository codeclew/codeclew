use clew::canonical;
use clew::cas::CasObject;
use clew::explanation::{
    CLAIM_INPUT_SCHEMA, ClaimAuthority, ClaimInput, ClaimInputDocument, ClaimPredicate,
};
use clew::thread_callables::RelationshipAuthority;
use clew::thread_flow::{
    FLOW_SLICE_SCHEMA, FlowBoundary, FlowBudgets, FlowCertainty, FlowControlRegion, FlowCounts,
    FlowDirection, FlowEdge, FlowNode, FlowNodeKind, FlowOrderAuthority, FlowRequest, FlowRootKind,
    FlowSlice, FlowStatus,
};
use clew::thread_flow_cfg::{
    LOCAL_CFG_PAYLOAD_SCHEMA, LocalCfgEdge, LocalCfgEdgeKind, LocalCfgNode, LocalCfgNodeRole,
    LocalCfgPayload, LocalCfgSupport,
};
use serde_json::json;
use std::process::Command;

const SOURCE: &str = "callable:com/acme/ProductService.save#jvm:save()V";
const TARGET: &str = "callable:com/acme/ProductRepository.insert#jvm:insert()V";

#[test]
fn core_computes_authority_and_republishes_deterministically() {
    let flow = flow(false);
    let flow_ref = flow_ref(&flow);
    let document = ClaimInputDocument {
        schema: CLAIM_INPUT_SCHEMA.into(),
        flow_id: flow.flow_id.clone(),
        claims: vec![
            claim(
                "call",
                ClaimPredicate::CallExists {
                    subject: SOURCE.into(),
                    object: TARGET.into(),
                },
                vec!["edge-call"],
                vec![],
            ),
            claim(
                "summary",
                ClaimPredicate::NarrativeSummary {
                    subject: SOURCE.into(),
                },
                vec!["edge-call"],
                vec![],
            ),
        ],
    };
    let first = clew::explanation::build(&flow, &flow_ref, document.clone()).unwrap();
    let second = clew::explanation::build(&flow, &flow_ref, document).unwrap();
    assert_eq!(first, second);
    assert!(
        first
            .bundle
            .claims
            .iter()
            .any(|claim| claim.authority == ClaimAuthority::CompilerProven)
    );
    assert!(
        first
            .bundle
            .claims
            .iter()
            .any(|claim| claim.authority == ClaimAuthority::AgentInferred)
    );
    clew::explanation::verify_prepared(&flow, &first).unwrap();
}

#[test]
fn cfg_branch_and_reachability_claims_are_checked_by_core() {
    let flow = flow(false);
    let flow_ref = flow_ref(&flow);
    let document = ClaimInputDocument {
        schema: CLAIM_INPUT_SCHEMA.into(),
        flow_id: flow.flow_id.clone(),
        claims: vec![
            claim(
                "branch",
                ClaimPredicate::BranchExists {
                    subject: SOURCE.into(),
                    region_id: "region-save".into(),
                    branch_kind: LocalCfgEdgeKind::True,
                },
                vec!["region-save"],
                vec![],
            ),
            claim(
                "order",
                ClaimPredicate::OrderedBefore {
                    subject: SOURCE.into(),
                    region_id: "region-save".into(),
                    before_node_id: 0,
                    after_node_id: 2,
                },
                vec!["region-save"],
                vec![],
            ),
        ],
    };
    let prepared = clew::explanation::build(&flow, &flow_ref, document).unwrap();
    assert!(prepared.bundle.claims.iter().any(|claim| {
        claim.predicate.kind() == clew::explanation::ExplanationPredicateKind::BranchExists
            && claim.authority == ClaimAuthority::CompilerProven
    }));
    assert!(prepared.bundle.claims.iter().any(|claim| {
        claim.predicate.kind() == clew::explanation::ExplanationPredicateKind::OrderedBefore
            && claim.authority == ClaimAuthority::StaticDerived
    }));

    let reversed = ClaimInputDocument {
        schema: CLAIM_INPUT_SCHEMA.into(),
        flow_id: flow.flow_id.clone(),
        claims: vec![claim(
            "bad-order",
            ClaimPredicate::OrderedBefore {
                subject: SOURCE.into(),
                region_id: "region-save".into(),
                before_node_id: 2,
                after_node_id: 0,
            },
            vec!["region-save"],
            vec![],
        )],
    };
    assert!(clew::explanation::build(&flow, &flow_ref, reversed).is_err());
}

#[test]
fn relevant_boundary_must_be_present_and_caps_authority_to_unknown() {
    let flow = flow(true);
    let flow_ref = flow_ref(&flow);
    let omitted = ClaimInputDocument {
        schema: CLAIM_INPUT_SCHEMA.into(),
        flow_id: flow.flow_id.clone(),
        claims: vec![claim(
            "call",
            ClaimPredicate::CallExists {
                subject: SOURCE.into(),
                object: TARGET.into(),
            },
            vec!["edge-call"],
            vec![],
        )],
    };
    assert!(clew::explanation::build(&flow, &flow_ref, omitted).is_err());

    let supplied = ClaimInputDocument {
        schema: CLAIM_INPUT_SCHEMA.into(),
        flow_id: flow.flow_id.clone(),
        claims: vec![claim(
            "call",
            ClaimPredicate::CallExists {
                subject: SOURCE.into(),
                object: TARGET.into(),
            },
            vec!["edge-call"],
            vec!["boundary-source"],
        )],
    };
    let prepared = clew::explanation::build(&flow, &flow_ref, supplied).unwrap();
    assert_eq!(prepared.bundle.claims[0].authority, ClaimAuthority::Unknown);
}

#[test]
fn invented_support_agent_authority_and_premature_handoff_fail_closed() {
    let flow = flow(false);
    let flow_ref = flow_ref(&flow);
    let invented = ClaimInputDocument {
        schema: CLAIM_INPUT_SCHEMA.into(),
        flow_id: flow.flow_id.clone(),
        claims: vec![claim(
            "invented",
            ClaimPredicate::CallExists {
                subject: SOURCE.into(),
                object: TARGET.into(),
            },
            vec!["edge-invented"],
            vec![],
        )],
    };
    assert!(clew::explanation::build(&flow, &flow_ref, invented).is_err());

    let handoff = ClaimInputDocument {
        schema: CLAIM_INPUT_SCHEMA.into(),
        flow_id: flow.flow_id.clone(),
        claims: vec![claim(
            "handoff",
            ClaimPredicate::ComponentHandoff {
                subject: "product".into(),
                object: "outbox".into(),
            },
            vec!["edge-call"],
            vec![],
        )],
    };
    assert!(clew::explanation::build(&flow, &flow_ref, handoff).is_err());

    let malicious = canonical::bytes(&json!({
        "schema": CLAIM_INPUT_SCHEMA,
        "flowId": flow.flow_id,
        "claims": [{
            "localId": "raise-authority",
            "locale": "ru",
            "text": "Надёжно",
            "predicate": {"kind":"NARRATIVE_SUMMARY","subject":SOURCE},
            "supportRefs":["edge-call"],
            "authority":"COMPILER_PROVEN"
        }]
    }))
    .unwrap();
    assert!(clew::explanation::parse_claim_document(&malicious).is_err());
}

#[test]
fn explain_cli_requires_explicit_thread_flow_and_inert_claim_file() {
    let output = Command::new(env!("CARGO_BIN_EXE_clew"))
        .args(["thread", "explain", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    for option in ["--thread", "--flow", "--claims"] {
        assert!(help.contains(option));
    }
    for args in [
        &["plan", "explain"][..],
        &["task-run", "explain"][..],
        &["thread", "publish"][..],
    ] {
        assert!(
            !Command::new(env!("CARGO_BIN_EXE_clew"))
                .args(args)
                .output()
                .unwrap()
                .status
                .success()
        );
    }
}

#[test]
fn empty_duplicate_stale_and_non_nfc_claims_are_rejected() {
    let flow = flow(false);
    let flow_ref = flow_ref(&flow);
    let empty = ClaimInputDocument {
        schema: CLAIM_INPUT_SCHEMA.into(),
        flow_id: flow.flow_id.clone(),
        claims: vec![],
    };
    assert!(clew::explanation::build(&flow, &flow_ref, empty).is_err());

    let base = claim(
        "same",
        ClaimPredicate::NarrativeSummary {
            subject: SOURCE.into(),
        },
        vec!["edge-call"],
        vec![],
    );
    let duplicate = ClaimInputDocument {
        schema: CLAIM_INPUT_SCHEMA.into(),
        flow_id: flow.flow_id.clone(),
        claims: vec![base.clone(), base],
    };
    assert!(clew::explanation::build(&flow, &flow_ref, duplicate).is_err());

    let stale = ClaimInputDocument {
        schema: CLAIM_INPUT_SCHEMA.into(),
        flow_id:
            "thread-flow:sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
                .into(),
        claims: vec![claim(
            "stale",
            ClaimPredicate::NarrativeSummary {
                subject: SOURCE.into(),
            },
            vec!["edge-call"],
            vec![],
        )],
    };
    assert!(clew::explanation::build(&flow, &flow_ref, stale).is_err());

    let mut non_nfc = claim(
        "unicode",
        ClaimPredicate::NarrativeSummary {
            subject: SOURCE.into(),
        },
        vec!["edge-call"],
        vec![],
    );
    non_nfc.text = "cafe\u{301}".into();
    let non_nfc = ClaimInputDocument {
        schema: CLAIM_INPUT_SCHEMA.into(),
        flow_id: flow.flow_id.clone(),
        claims: vec![non_nfc],
    };
    assert!(clew::explanation::build(&flow, &flow_ref, non_nfc).is_err());
}

fn claim(
    local_id: &str,
    predicate: ClaimPredicate,
    support_refs: Vec<&str>,
    boundary_refs: Vec<&str>,
) -> ClaimInput {
    ClaimInput {
        local_id: local_id.into(),
        locale: "ru".into(),
        text: format!("Описание {local_id}"),
        predicate,
        support_refs: support_refs.into_iter().map(str::to_owned).collect(),
        boundary_refs: boundary_refs.into_iter().map(str::to_owned).collect(),
    }
}

fn flow(with_boundary: bool) -> FlowSlice {
    let graph = clew::thread_flow_cfg::seal(LocalCfgPayload {
        schema: "local-cfg/0.1".into(),
        graph_id: String::new(),
        owner_symbol_identity: SOURCE.into(),
        file: "src/ProductService.kt".into(),
        compiler_graph_name: "com/acme/ProductService.save".into(),
        provider: "K2_FIR_CFG".into(),
        source_provenance: "COMPILER_UTF16_RANGE_TO_UTF8_BYTES".into(),
        nodes: vec![
            cfg_node(0, LocalCfgNodeRole::Entry),
            cfg_node(1, LocalCfgNodeRole::Decision),
            cfg_node(2, LocalCfgNodeRole::Return),
        ],
        edges: vec![
            LocalCfgEdge {
                source_node_id: 0,
                target_node_id: 1,
                kind: LocalCfgEdgeKind::Next,
                label: None,
            },
            LocalCfgEdge {
                source_node_id: 1,
                target_node_id: 2,
                kind: LocalCfgEdgeKind::True,
                label: Some("saved".into()),
            },
        ],
    })
    .unwrap();
    let boundaries = if with_boundary {
        vec![FlowBoundary {
            boundary_id: "boundary-source".into(),
            code: "VERIFY_TRANSACTION_BOUNDARY".into(),
            subject: SOURCE.into(),
            required_checks: vec!["VERIFY_TRANSACTION_BOUNDARY".into()],
            support_refs: vec![],
        }]
    } else {
        Vec::new()
    };
    FlowSlice {
        schema: FLOW_SLICE_SCHEMA.into(),
        flow_id: "thread-flow:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            .into(),
        request: FlowRequest {
            thread_id: "thread:test".into(),
            thread_authority_digest: digest("thread"),
            fact_set_id: "thread-callables:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            fact_set_authority_digest: digest("facts"),
            pair_id: "product-outbox".into(),
            member_alias: "product".into(),
            root_kind: FlowRootKind::FullSymbol,
            root: SOURCE.into(),
            direction: FlowDirection::Downstream,
            budgets: FlowBudgets::frozen(4).unwrap(),
        },
        root_node_id: "node-source".into(),
        nodes: vec![node("node-source", SOURCE, 0), node("node-target", TARGET, 1)],
        edges: vec![FlowEdge {
            edge_id: "edge-call".into(),
            source_node_id: "node-source".into(),
            target_node_id: "node-target".into(),
            relation_kind: "CALLS".into(),
            relationship_authority:
                RelationshipAuthority::VerifiedSameSnapshotCompilationDependency,
            order_authority: FlowOrderAuthority::CompilerCfg,
            cfg_graph_id: Some(graph.graph_id.clone()),
            cfg_node_ids: vec![0, 1, 2],
            support_refs: vec![],
        }],
        boundaries: boundaries.clone(),
        control_flow_regions: vec![FlowControlRegion {
            region_id: "region-save".into(),
            owner_node_id: "node-source".into(),
            graph,
            support: LocalCfgSupport {
                member_alias: "product".into(),
                compilation_id: ":/main".into(),
                generation_ref: object("codeclew-generation-manifest/2.0", "generation"),
                payload_ref: object(LOCAL_CFG_PAYLOAD_SCHEMA, "cfg"),
            },
        }],
        counts: FlowCounts {
            nodes: 2,
            edges: 1,
            boundaries: boundaries.len(),
            control_flow_regions: 1,
        },
        status: FlowStatus::Complete,
        certainty: if with_boundary {
            FlowCertainty::Unsure
        } else {
            FlowCertainty::Verified
        },
        verification_obligations: if with_boundary {
            vec!["VERIFY_TRANSACTION_BOUNDARY".into()]
        } else {
            Vec::new()
        },
        parent_fact_shards: vec![],
    }
}

fn node(node_id: &str, symbol: &str, depth: usize) -> FlowNode {
    FlowNode {
        node_id: node_id.into(),
        node_kind: FlowNodeKind::Callable,
        member_alias: "product".into(),
        repository_namespace: "repo:product".into(),
        symbol_identity: symbol.into(),
        depth,
        order_authority: FlowOrderAuthority::CompilerCfg,
        support_refs: vec![],
    }
}

fn cfg_node(node_id: u64, role: LocalCfgNodeRole) -> LocalCfgNode {
    LocalCfgNode {
        node_id,
        role,
        source: Some(clew::thread_flow_cfg::LocalCfgSourceRange {
            start: node_id,
            end: node_id + 1,
        }),
    }
}

fn flow_ref(flow: &FlowSlice) -> CasObject {
    CasObject::for_bytes(FLOW_SLICE_SCHEMA, &canonical::bytes(flow).unwrap()).unwrap()
}

fn object(schema: &str, label: &str) -> CasObject {
    CasObject::for_bytes(schema, label.as_bytes()).unwrap()
}

fn digest(label: &str) -> String {
    canonical::hash(&label).unwrap()
}
