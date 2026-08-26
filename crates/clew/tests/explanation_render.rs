use clew::canonical;
use clew::cas::CasObject;
use clew::explanation::{
    ClaimAuthority, ClaimPredicate, EXPLANATION_BUNDLE_SCHEMA, ExplanationBundle, ExplanationClaim,
};
use clew::explanation_render::{DetailLevel, RenderFormat};
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
use std::process::Command;

const SOURCE: &str = "callable:com/acme/ProductService.save#jvm:save()V";
const TARGET: &str = "callable:com/acme/ProductRepository.insert#jvm:insert()V";

#[test]
fn five_levels_share_claim_authority_boundaries_and_markdown_semantics() {
    let flow = flow();
    let bundle = bundle(&flow);
    for detail in [
        DetailLevel::Summary,
        DetailLevel::Scenario,
        DetailLevel::Technical,
        DetailLevel::Evidence,
        DetailLevel::Compiler,
    ] {
        let json =
            clew::explanation_render::render(&bundle, &flow, detail, RenderFormat::Json).unwrap();
        let markdown =
            clew::explanation_render::render(&bundle, &flow, detail, RenderFormat::Markdown)
                .unwrap();
        assert_eq!(json["explanationId"], markdown["explanationId"]);
        assert_eq!(json["semanticDigest"], markdown["semanticDigest"]);
        assert_eq!(json["truncated"], markdown["truncated"]);
        assert_eq!(json["boundaries"].as_array().unwrap().len(), 1);
        let content = markdown["content"].as_str().unwrap();
        assert!(content.contains("VERIFY_TRANSACTION_BOUNDARY"));
        for claim in json["claims"].as_array().unwrap() {
            assert!(content.contains(claim["claimId"].as_str().unwrap()));
            assert!(content.contains(claim["authority"].as_str().unwrap()));
        }
    }
}

#[test]
fn markdown_escapes_agent_text_and_keeps_stable_expand_refs() {
    let flow = flow();
    let mut bundle = bundle(&flow);
    bundle.claims[0].text = "*unsafe* [label] <tag>".into();
    let value = clew::explanation_render::render(
        &bundle,
        &flow,
        DetailLevel::Evidence,
        RenderFormat::Markdown,
    )
    .unwrap();
    let content = value["content"].as_str().unwrap();
    assert!(content.contains("\\*unsafe\\*"));
    assert!(content.contains("\\[label\\]"));
    assert!(content.contains("\\<tag\\>"));
    assert!(content.contains("edge-call"));
}

#[test]
fn large_optional_sections_truncate_without_losing_boundaries() {
    let flow = flow();
    let mut bundle = bundle(&flow);
    bundle.claims = (0..200)
        .map(|index| ExplanationClaim {
            claim_id: format!("claim-{index:04}"),
            local_id: format!("claim-{index:04}"),
            locale: "en".into(),
            text: "x".repeat(4_096),
            predicate: ClaimPredicate::NarrativeSummary {
                subject: SOURCE.into(),
            },
            authority: ClaimAuthority::AgentInferred,
            support_refs: vec!["edge-call".into()],
            boundary_refs: vec!["boundary-transaction".into()],
        })
        .collect();
    let value =
        clew::explanation_render::render(&bundle, &flow, DetailLevel::Compiler, RenderFormat::Json)
            .unwrap();
    assert_eq!(value["truncated"], true);
    assert_eq!(value["boundaries"].as_array().unwrap().len(), 1);
    assert!(canonical::bytes(&value).unwrap().len() < 64 * 1024);
}

#[test]
fn truncated_technical_render_keeps_both_pair_member_lanes() {
    let mut flow = flow();
    for index in 0..1_500 {
        flow.nodes.push(FlowNode {
            node_id: format!("node-product-{index:04}"),
            node_kind: FlowNodeKind::Callable,
            member_alias: "product".into(),
            service_alias: "product-service".into(),
            repository_namespace: "repo:product".into(),
            symbol_identity: format!("callable:sample/Product.extra{index}#jvm:()V"),
            depth: 1,
            order_authority: FlowOrderAuthority::UnorderedStaticRelation,
            support_refs: vec![],
        });
    }
    flow.nodes.push(FlowNode {
        node_id: "node-outbox-boundary".into(),
        node_kind: FlowNodeKind::Boundary,
        member_alias: "outbox".into(),
        service_alias: "outbox-service".into(),
        repository_namespace: "repo:outbox".into(),
        symbol_identity: "sample/Outbox.publish".into(),
        depth: 1,
        order_authority: FlowOrderAuthority::Unknown,
        support_refs: vec![],
    });
    flow.counts.nodes = flow.nodes.len();
    let value = clew::explanation_render::render(
        &bundle(&flow),
        &flow,
        DetailLevel::Technical,
        RenderFormat::Json,
    )
    .unwrap();
    assert_eq!(value["truncated"], true);
    let members = value["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|node| node["memberAlias"].as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert!(members.contains("product"));
    assert!(members.contains("outbox"));
}

#[test]
fn render_cli_closes_detail_and_format_enums() {
    let binary = env!("CARGO_BIN_EXE_clew");
    let help = Command::new(binary)
        .args(["thread", "render", "--help"])
        .output()
        .unwrap();
    assert!(help.status.success());
    let text = String::from_utf8(help.stdout).unwrap();
    for option in ["--thread", "--explanation", "--detail", "--format"] {
        assert!(text.contains(option));
    }
    let invalid = Command::new(binary)
        .args([
            "thread",
            "render",
            "--thread",
            "thread:test",
            "--explanation",
            "thread-explanation:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "--detail",
            "debug",
            "--format",
            "html",
        ])
        .output()
        .unwrap();
    assert!(!invalid.status.success());
}

fn bundle(flow: &FlowSlice) -> ExplanationBundle {
    ExplanationBundle {
        schema: EXPLANATION_BUNDLE_SCHEMA.into(),
        explanation_id:
            "thread-explanation:sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"
                .into(),
        thread_id: flow.request.thread_id.clone(),
        thread_authority_digest: flow.request.thread_authority_digest.clone(),
        fact_set_id: flow.request.fact_set_id.clone(),
        fact_set_authority_digest: flow.request.fact_set_authority_digest.clone(),
        flow_id: flow.flow_id.clone(),
        flow_slice_ref: CasObject::for_bytes(
            FLOW_SLICE_SCHEMA,
            &canonical::bytes(flow).unwrap(),
        )
        .unwrap(),
        claims_input_digest: digest("claims"),
        claims: vec![
            ExplanationClaim {
                claim_id: "claim-summary".into(),
                local_id: "summary".into(),
                locale: "en".into(),
                text: "The product is saved".into(),
                predicate: ClaimPredicate::NarrativeSummary {
                    subject: SOURCE.into(),
                },
                authority: ClaimAuthority::AgentInferred,
                support_refs: vec!["edge-call".into()],
                boundary_refs: vec!["boundary-transaction".into()],
            },
            ExplanationClaim {
                claim_id: "claim-branch".into(),
                local_id: "branch".into(),
                locale: "en".into(),
                text: "A success branch exists".into(),
                predicate: ClaimPredicate::BranchExists {
                    subject: SOURCE.into(),
                    region_id: "region-save".into(),
                    branch_kind: LocalCfgEdgeKind::True,
                },
                authority: ClaimAuthority::CompilerProven,
                support_refs: vec!["region-save".into()],
                boundary_refs: vec![],
            },
            ExplanationClaim {
                claim_id: "claim-call".into(),
                local_id: "call".into(),
                locale: "en".into(),
                text: "Save calls insert".into(),
                predicate: ClaimPredicate::CallExists {
                    subject: SOURCE.into(),
                    object: TARGET.into(),
                },
                authority: ClaimAuthority::CompilerProven,
                support_refs: vec!["edge-call".into()],
                boundary_refs: vec![],
            },
        ],
        boundaries: flow.boundaries.clone(),
        verification_obligations: flow.verification_obligations.clone(),
    }
}

fn flow() -> FlowSlice {
    let graph = clew::thread_flow_cfg::seal(LocalCfgPayload {
        schema: "local-cfg/0.1".into(),
        graph_id: String::new(),
        owner_symbol_identity: SOURCE.into(),
        file: "src/ProductService.kt".into(),
        compiler_graph_name: "ProductService.save".into(),
        provider: "K2_FIR_CFG".into(),
        source_provenance: "COMPILER_UTF16_RANGE_TO_UTF8_BYTES".into(),
        nodes: vec![
            node_cfg(0, LocalCfgNodeRole::Entry),
            node_cfg(1, LocalCfgNodeRole::Decision),
            node_cfg(2, LocalCfgNodeRole::Return),
        ],
        edges: vec![
            edge_cfg(0, 1, LocalCfgEdgeKind::Next),
            edge_cfg(1, 2, LocalCfgEdgeKind::True),
        ],
    })
    .unwrap();
    let boundaries = vec![FlowBoundary {
        boundary_id: "boundary-transaction".into(),
        code: "VERIFY_TRANSACTION_BOUNDARY".into(),
        subject: SOURCE.into(),
        required_checks: vec!["VERIFY_TRANSACTION_BOUNDARY".into()],
        support_refs: vec![],
    }];
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
        nodes: vec![flow_node("node-source", SOURCE, 0), flow_node("node-target", TARGET, 1)],
        edges: vec![FlowEdge {
            edge_id: "edge-call".into(),
            source_node_id: "node-source".into(),
            target_node_id: "node-target".into(),
            source_member_alias: "product".into(),
            source_service_alias: "product-service".into(),
            source_repository_namespace: "repo:product".into(),
            target_member_alias: "product".into(),
            target_service_alias: "product-service".into(),
            target_repository_namespace: "repo:product".into(),
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
            boundaries: 1,
            control_flow_regions: 1,
        },
        status: FlowStatus::Complete,
        certainty: FlowCertainty::Unsure,
        verification_obligations: vec!["VERIFY_TRANSACTION_BOUNDARY".into()],
        parent_fact_shards: vec![],
    }
}

fn flow_node(id: &str, symbol: &str, depth: usize) -> FlowNode {
    FlowNode {
        node_id: id.into(),
        node_kind: FlowNodeKind::Callable,
        member_alias: "product".into(),
        service_alias: "product-service".into(),
        repository_namespace: "repo:product".into(),
        symbol_identity: symbol.into(),
        depth,
        order_authority: FlowOrderAuthority::CompilerCfg,
        support_refs: vec![],
    }
}

fn node_cfg(node_id: u64, role: LocalCfgNodeRole) -> LocalCfgNode {
    LocalCfgNode {
        node_id,
        role,
        source: None,
    }
}

fn edge_cfg(source: u64, target: u64, kind: LocalCfgEdgeKind) -> LocalCfgEdge {
    LocalCfgEdge {
        source_node_id: source,
        target_node_id: target,
        kind,
        label: None,
    }
}

fn object(schema: &str, label: &str) -> CasObject {
    CasObject::for_bytes(schema, label.as_bytes()).unwrap()
}

fn digest(label: &str) -> String {
    canonical::hash(&label).unwrap()
}
