//! Pure construction of a bounded, exact-root static flow slice.
//!
//! The builder consumes only a verified retained callable fact set. It has no
//! filesystem, process, compiler, or publication capability.

use crate::canonical;
use crate::cas::CasObject;
use crate::error::{ClewError, ErrorCode};
use crate::thread_callables::{
    CallableFact, CallableFactProvenance, CallableFactSetCertainty, CallableFactShard,
    PreparedCallableFactSet, RelationshipAuthority, SourceAnchor, TargetResolution,
};
use crate::thread_flow_cfg::{LocalCfgCatalog, LocalCfgPayload, LocalCfgSupport};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub const FLOW_SLICE_SCHEMA: &str = "codeclew-flow-slice/0.1";
pub const FLOW_SLICE_PROJECTION_SCHEMA: &str = "codeclew-flow-slice-projection/0.1";
pub const MAX_FLOW_DEPTH: usize = 32;
pub const MAX_FLOW_NODES: usize = 4_096;
pub const MAX_FLOW_EDGES: usize = 8_192;
pub const MAX_FLOW_BOUNDARIES: usize = 4_096;
pub const MAX_FLOW_STDOUT_BYTES: usize = 64 * 1024;
pub const MAX_FLOW_SLICE_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FlowRootKind {
    FullSymbol,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FlowDirection {
    Downstream,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FlowOrderAuthority {
    UnorderedStaticRelation,
    CompilerCfg,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FlowStatus {
    Complete,
    Truncated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FlowCertainty {
    Verified,
    Unsure,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FlowBudgets {
    pub max_depth: usize,
    pub max_nodes: usize,
    pub max_edges: usize,
    pub max_boundaries: usize,
    pub max_stdout_bytes: usize,
    pub max_slice_bytes: usize,
}

impl FlowBudgets {
    pub fn frozen(max_depth: usize) -> Result<Self, ClewError> {
        if max_depth == 0 || max_depth > MAX_FLOW_DEPTH {
            return Err(invalid("flow max depth must be between one and 32"));
        }
        Ok(Self {
            max_depth,
            max_nodes: MAX_FLOW_NODES,
            max_edges: MAX_FLOW_EDGES,
            max_boundaries: MAX_FLOW_BOUNDARIES,
            max_stdout_bytes: MAX_FLOW_STDOUT_BYTES,
            max_slice_bytes: MAX_FLOW_SLICE_BYTES,
        })
    }

    fn validate(&self) -> Result<(), ClewError> {
        if self.max_depth == 0
            || self.max_depth > MAX_FLOW_DEPTH
            || self.max_nodes != MAX_FLOW_NODES
            || self.max_edges != MAX_FLOW_EDGES
            || self.max_boundaries != MAX_FLOW_BOUNDARIES
            || self.max_stdout_bytes != MAX_FLOW_STDOUT_BYTES
            || self.max_slice_bytes != MAX_FLOW_SLICE_BYTES
        {
            return Err(invalid("flow budgets differ from the frozen profile"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FlowRequest {
    pub thread_id: String,
    pub thread_authority_digest: String,
    pub fact_set_id: String,
    pub fact_set_authority_digest: String,
    pub pair_id: String,
    pub member_alias: String,
    pub root_kind: FlowRootKind,
    pub root: String,
    pub direction: FlowDirection,
    pub budgets: FlowBudgets,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FlowSupportRef {
    pub fact_id: String,
    pub fact_shard_ref: CasObject,
    pub provenance: CallableFactProvenance,
    pub input_payload_ref: CasObject,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceAnchor>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FlowNodeKind {
    Callable,
    Boundary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FlowNode {
    pub node_id: String,
    pub node_kind: FlowNodeKind,
    pub member_alias: String,
    pub repository_namespace: String,
    pub symbol_identity: String,
    pub depth: usize,
    pub order_authority: FlowOrderAuthority,
    pub support_refs: Vec<FlowSupportRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FlowEdge {
    pub edge_id: String,
    pub source_node_id: String,
    pub target_node_id: String,
    pub relation_kind: String,
    pub relationship_authority: RelationshipAuthority,
    pub order_authority: FlowOrderAuthority,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cfg_graph_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cfg_node_ids: Vec<u64>,
    pub support_refs: Vec<FlowSupportRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FlowControlRegion {
    pub region_id: String,
    pub owner_node_id: String,
    pub graph: LocalCfgPayload,
    pub support: LocalCfgSupport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FlowBoundary {
    pub boundary_id: String,
    pub code: String,
    pub subject: String,
    pub required_checks: Vec<String>,
    pub support_refs: Vec<FlowSupportRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FlowCounts {
    pub nodes: usize,
    pub edges: usize,
    pub boundaries: usize,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub control_flow_regions: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FlowSlice {
    pub schema: String,
    pub flow_id: String,
    pub request: FlowRequest,
    pub root_node_id: String,
    pub nodes: Vec<FlowNode>,
    pub edges: Vec<FlowEdge>,
    pub boundaries: Vec<FlowBoundary>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub control_flow_regions: Vec<FlowControlRegion>,
    pub counts: FlowCounts,
    pub status: FlowStatus,
    pub certainty: FlowCertainty,
    pub verification_obligations: Vec<String>,
    pub parent_fact_shards: Vec<CasObject>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FlowSliceProjection {
    pub schema: String,
    pub flow_id: String,
    pub root: String,
    pub member_alias: String,
    pub root_node_id: String,
    pub counts: FlowCounts,
    pub status: FlowStatus,
    pub certainty: FlowCertainty,
    pub order_authority: FlowOrderAuthority,
    pub verification_obligations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedFlowSlice {
    pub slice: FlowSlice,
    pub slice_bytes: Vec<u8>,
    pub slice_ref: CasObject,
    pub projection: FlowSliceProjection,
}

#[derive(Clone)]
struct IndexedFact {
    fact: CallableFact,
    shard_ref: CasObject,
}

impl IndexedFact {
    fn support(&self) -> FlowSupportRef {
        let provenance = match &self.fact {
            CallableFact::Declaration(row) => &row.provenance,
            CallableFact::Use(row) => &row.provenance,
            CallableFact::Boundary(row) => &row.provenance,
        };
        FlowSupportRef {
            fact_id: self.fact.fact_id().into(),
            fact_shard_ref: self.shard_ref.clone(),
            provenance: provenance.clone(),
            input_payload_ref: provenance.input_payload_ref.clone(),
            source: provenance.source.clone(),
        }
    }
}

pub fn build(
    request: FlowRequest,
    fact_set: &PreparedCallableFactSet,
) -> Result<PreparedFlowSlice, ClewError> {
    build_internal(request, fact_set, None)
}

pub fn build_with_cfg(
    request: FlowRequest,
    fact_set: &PreparedCallableFactSet,
    cfg: &LocalCfgCatalog,
) -> Result<PreparedFlowSlice, ClewError> {
    build_internal(request, fact_set, Some(cfg))
}

fn build_internal(
    request: FlowRequest,
    fact_set: &PreparedCallableFactSet,
    cfg: Option<&LocalCfgCatalog>,
) -> Result<PreparedFlowSlice, ClewError> {
    validate_request(&request, fact_set)?;
    let facts = read_facts(fact_set)?;
    let declarations = facts
        .iter()
        .filter_map(|fact| match &fact.fact {
            CallableFact::Declaration(row) if row.exact_eligible => Some((
                (
                    row.provenance.member_alias.clone(),
                    row.symbol_identity.clone(),
                ),
                fact.clone(),
            )),
            _ => None,
        })
        .fold(BTreeMap::<_, Vec<_>>::new(), |mut map, (key, fact)| {
            map.entry(key).or_default().push(fact);
            map
        });
    let root_key = (request.member_alias.clone(), request.root.clone());
    let roots = declarations
        .get(&root_key)
        .ok_or_else(|| invalid("exact flow root declaration was not found"))?;
    if roots.len() != 1 {
        return Err(invalid("exact flow root declaration is ambiguous"));
    }

    let callable_families = facts
        .iter()
        .filter_map(|fact| match &fact.fact {
            CallableFact::Declaration(row) if row.exact_eligible => {
                row.compiler_callable_id.as_ref().map(|callable_id| {
                    (
                        (row.provenance.member_alias.clone(), callable_id.clone()),
                        fact.clone(),
                    )
                })
            }
            _ => None,
        })
        .fold(BTreeMap::<_, Vec<_>>::new(), |mut map, (key, fact)| {
            map.entry(key).or_default().push(fact);
            map
        });

    let uses = facts
        .iter()
        .filter_map(|fact| match &fact.fact {
            CallableFact::Use(row) => Some((
                (
                    row.provenance.member_alias.clone(),
                    row.source_owner.clone(),
                ),
                fact.clone(),
            )),
            _ => None,
        })
        .fold(BTreeMap::<_, Vec<_>>::new(), |mut map, (key, fact)| {
            map.entry(key).or_default().push(fact);
            map
        });

    let mut nodes = BTreeMap::<String, FlowNode>::new();
    let mut edges = BTreeMap::<String, FlowEdge>::new();
    let mut boundaries = BTreeMap::<String, FlowBoundary>::new();
    let mut obligations = BTreeSet::<String>::new();
    let mut truncated = false;
    let root_node = callable_node(&roots[0], 0)?;
    let root_node_id = root_node.node_id.clone();
    nodes.insert(root_node.node_id.clone(), root_node);
    let mut queue = VecDeque::from([(request.root.clone(), 0usize)]);
    let mut visited = BTreeSet::from([request.root.clone()]);

    while let Some((source_symbol, depth)) = queue.pop_front() {
        let Some(source_node_id) = node_for_symbol(&nodes, &source_symbol) else {
            return Err(corrupt("flow traversal lost a visited callable node"));
        };
        let source_declarations = declarations
            .get(&(request.member_alias.clone(), source_symbol.clone()))
            .ok_or_else(|| corrupt("visited flow callable lost its declaration"))?;
        let source_declaration = match &source_declarations[0].fact {
            CallableFact::Declaration(row) => row,
            _ => unreachable!(),
        };
        let Some(source_callable_id) = source_declaration.compiler_callable_id.as_ref() else {
            add_boundary(
                &mut boundaries,
                &mut obligations,
                &request,
                "MISSING_SOURCE_CALLABLE_ID",
                &source_symbol,
                vec!["VERIFY_EXACT_SOURCE_CALLABLE_ID".into()],
                vec![source_declarations[0].support()],
            )?;
            continue;
        };
        if callable_families
            .get(&(request.member_alias.clone(), source_callable_id.clone()))
            .is_none_or(|family| family.len() != 1)
        {
            add_boundary(
                &mut boundaries,
                &mut obligations,
                &request,
                "AMBIGUOUS_SOURCE_OWNER_OVERLOAD",
                source_callable_id,
                vec!["DISAMBIGUATE_RELATION_SOURCE_OVERLOAD".into()],
                vec![source_declarations[0].support()],
            )?;
            continue;
        }
        let outgoing = uses
            .get(&(request.member_alias.clone(), source_callable_id.clone()))
            .cloned()
            .unwrap_or_default();
        if depth >= request.budgets.max_depth && !outgoing.is_empty() {
            truncated = true;
            add_boundary(
                &mut boundaries,
                &mut obligations,
                &request,
                "FLOW_DEPTH_TRUNCATED",
                &source_symbol,
                vec!["INCREASE_MAX_DEPTH_OR_SELECT_NARROWER_ROOT".into()],
                Vec::new(),
            )?;
            continue;
        }
        for indexed in outgoing {
            let CallableFact::Use(row) = &indexed.fact else {
                unreachable!();
            };
            let exact_target = row
                .target_symbol_identity
                .as_ref()
                .filter(|_| row.target_resolution == TargetResolution::ExactSymbol);
            let same_member = row.relationship_authority
                == RelationshipAuthority::VerifiedSameSnapshotCompilationDependency
                && row
                    .target_repository_namespace
                    .as_deref()
                    .is_some_and(|namespace| namespace == row.provenance.repository_namespace);
            if !same_member {
                add_boundary(
                    &mut boundaries,
                    &mut obligations,
                    &request,
                    "CROSS_MEMBER_NOT_EXPANDED",
                    exact_target
                        .map(String::as_str)
                        .unwrap_or(&row.target_callable_id),
                    vec!["VERIFY_SELECTED_MEMBER_OR_ENABLE_PAIR_FLOW".into()],
                    vec![indexed.support()],
                )?;
                continue;
            }
            let Some(target_symbol) = exact_target else {
                add_boundary(
                    &mut boundaries,
                    &mut obligations,
                    &request,
                    "UNRESOLVED_TARGET",
                    &row.target_callable_id,
                    vec!["RESOLVE_EXACT_FULL_SYMBOL".into()],
                    vec![indexed.support()],
                )?;
                continue;
            };
            let target_key = (request.member_alias.clone(), target_symbol.clone());
            let Some(targets) = declarations.get(&target_key) else {
                add_boundary(
                    &mut boundaries,
                    &mut obligations,
                    &request,
                    "MISSING_TARGET_DECLARATION",
                    target_symbol,
                    vec!["VERIFY_TARGET_DECLARATION_IN_RETAINED_FACT_SET".into()],
                    vec![indexed.support()],
                )?;
                continue;
            };
            if targets.len() != 1 {
                add_boundary(
                    &mut boundaries,
                    &mut obligations,
                    &request,
                    "AMBIGUOUS_TARGET_DECLARATION",
                    target_symbol,
                    vec!["DISAMBIGUATE_TARGET_FULL_SYMBOL".into()],
                    vec![indexed.support()],
                )?;
                continue;
            }
            let target_node = callable_node(&targets[0], depth + 1)?;
            if !nodes.contains_key(&target_node.node_id) && nodes.len() >= request.budgets.max_nodes
            {
                truncated = true;
                add_boundary(
                    &mut boundaries,
                    &mut obligations,
                    &request,
                    "FLOW_NODE_BUDGET_TRUNCATED",
                    target_symbol,
                    vec!["SELECT_NARROWER_ROOT".into()],
                    vec![indexed.support()],
                )?;
                continue;
            }
            let target_node_id = target_node.node_id.clone();
            nodes.entry(target_node_id.clone()).or_insert(target_node);
            if edges.len() >= request.budgets.max_edges {
                truncated = true;
                add_boundary(
                    &mut boundaries,
                    &mut obligations,
                    &request,
                    "FLOW_EDGE_BUDGET_TRUNCATED",
                    &source_symbol,
                    vec!["SELECT_NARROWER_ROOT".into()],
                    vec![indexed.support()],
                )?;
                continue;
            }
            let edge_id = stable_id(
                "flow-edge",
                &json!({
                    "source": source_node_id,
                    "target": target_node_id,
                    "fact": row.fact_id,
                }),
            )?;
            let (cfg_graph_id, cfg_node_ids, order_authority) = match cfg {
                None => (
                    None,
                    Vec::new(),
                    FlowOrderAuthority::UnorderedStaticRelation,
                ),
                Some(catalog) => match catalog.get(&request.member_alias, &source_symbol) {
                    Some(graph) => match relation_cfg_nodes(row, &graph.payload) {
                        Some(node_ids) => (
                            Some(graph.payload.graph_id.clone()),
                            node_ids,
                            FlowOrderAuthority::CompilerCfg,
                        ),
                        None => {
                            add_boundary(
                                &mut boundaries,
                                &mut obligations,
                                &request,
                                "VERIFY_CONTROL_FLOW_ORDER",
                                &source_symbol,
                                vec!["VERIFY_CONTROL_FLOW_ORDER".into()],
                                vec![indexed.support()],
                            )?;
                            (None, Vec::new(), FlowOrderAuthority::Unknown)
                        }
                    },
                    None => (None, Vec::new(), FlowOrderAuthority::Unknown),
                },
            };
            edges.entry(edge_id.clone()).or_insert(FlowEdge {
                edge_id,
                source_node_id: source_node_id.clone(),
                target_node_id,
                relation_kind: row.relation_kind.clone(),
                relationship_authority: row.relationship_authority,
                order_authority,
                cfg_graph_id,
                cfg_node_ids,
                support_refs: vec![indexed.support()],
            });
            if visited.insert(target_symbol.clone()) {
                queue.push_back((target_symbol.clone(), depth + 1));
            }
        }
    }

    let missing_source_support = nodes
        .values()
        .flat_map(|node| {
            node.support_refs
                .iter()
                .filter(|support| support.source.is_none())
                .map(|support| (node.symbol_identity.clone(), support.clone()))
        })
        .chain(edges.values().flat_map(|edge| {
            edge.support_refs
                .iter()
                .filter(|support| support.source.is_none())
                .map(|support| (edge.source_node_id.clone(), support.clone()))
        }))
        .collect::<Vec<_>>();
    for (subject, support) in missing_source_support {
        add_boundary(
            &mut boundaries,
            &mut obligations,
            &request,
            "MISSING_SOURCE_ANCHOR",
            &subject,
            vec!["VERIFY_EXACT_SOURCE_ANCHOR".into()],
            vec![support],
        )?;
    }

    for indexed in facts {
        let CallableFact::Boundary(row) = &indexed.fact else {
            continue;
        };
        if row.provenance.member_alias != request.member_alias {
            continue;
        }
        let relevant = row
            .subject
            .as_ref()
            .is_some_and(|subject| visited.contains(subject));
        if relevant {
            add_boundary(
                &mut boundaries,
                &mut obligations,
                &request,
                &row.code,
                row.subject.as_deref().unwrap_or(&request.root),
                row.required_checks.clone(),
                vec![indexed.support()],
            )?;
        }
    }

    let mut control_flow_regions = Vec::new();
    if let Some(catalog) = cfg {
        for symbol in &visited {
            let Some(owner_node_id) = node_for_symbol(&nodes, symbol) else {
                continue;
            };
            match catalog.get(&request.member_alias, symbol) {
                Some(graph) => control_flow_regions.push(FlowControlRegion {
                    region_id: stable_id(
                        "flow-cfg-region",
                        &json!({
                            "ownerNodeId": owner_node_id,
                            "graphId": graph.payload.graph_id,
                            "support": graph.support,
                        }),
                    )?,
                    owner_node_id,
                    graph: graph.payload.clone(),
                    support: graph.support.clone(),
                }),
                None => add_boundary(
                    &mut boundaries,
                    &mut obligations,
                    &request,
                    "VERIFY_CONTROL_FLOW_ORDER",
                    symbol,
                    vec!["VERIFY_CONTROL_FLOW_ORDER".into()],
                    Vec::new(),
                )?,
            }
        }
    }

    let mut nodes = nodes.into_values().collect::<Vec<_>>();
    let mut edges = edges.into_values().collect::<Vec<_>>();
    let mut boundaries = boundaries.into_values().collect::<Vec<_>>();
    nodes.sort_by(|left, right| left.node_id.cmp(&right.node_id));
    edges.sort_by(|left, right| left.edge_id.cmp(&right.edge_id));
    boundaries.sort_by(|left, right| left.boundary_id.cmp(&right.boundary_id));
    control_flow_regions.sort_by(|left, right| left.region_id.cmp(&right.region_id));
    let certainty = if truncated
        || !boundaries.is_empty()
        || fact_set.authority.completeness.certainty == CallableFactSetCertainty::Unsure
    {
        FlowCertainty::Unsure
    } else {
        FlowCertainty::Verified
    };
    let status = if truncated {
        FlowStatus::Truncated
    } else {
        FlowStatus::Complete
    };
    let counts = FlowCounts {
        nodes: nodes.len(),
        edges: edges.len(),
        boundaries: boundaries.len(),
        control_flow_regions: control_flow_regions.len(),
    };
    let obligations = obligations.into_iter().collect::<Vec<_>>();
    let projection_order_authority = match cfg {
        None => FlowOrderAuthority::UnorderedStaticRelation,
        Some(_)
            if counts.control_flow_regions > 0
                && edges
                    .iter()
                    .all(|edge| edge.order_authority == FlowOrderAuthority::CompilerCfg) =>
        {
            FlowOrderAuthority::CompilerCfg
        }
        Some(_) => FlowOrderAuthority::Unknown,
    };
    let parent_fact_shards = fact_set
        .authority
        .fact_shards
        .iter()
        .map(|reference| reference.object.clone())
        .collect::<Vec<_>>();
    let identity = json!({
        "schema": FLOW_SLICE_SCHEMA,
        "request": request,
        "rootNodeId": root_node_id,
        "nodes": nodes,
        "edges": edges,
        "boundaries": boundaries,
        "controlFlowRegions": control_flow_regions,
        "status": status,
        "certainty": certainty,
        "verificationObligations": obligations,
        "parentFactShards": parent_fact_shards,
    });
    let flow_id = format!(
        "thread-flow:{}",
        canonical::hash(&identity).map_err(internal)?
    );
    let slice = FlowSlice {
        schema: FLOW_SLICE_SCHEMA.into(),
        flow_id: flow_id.clone(),
        request: request.clone(),
        root_node_id: root_node_id.clone(),
        nodes,
        edges,
        boundaries,
        control_flow_regions,
        counts: counts.clone(),
        status,
        certainty,
        verification_obligations: obligations.clone(),
        parent_fact_shards,
    };
    let slice_bytes = canonical::bytes(&slice).map_err(internal)?;
    if slice_bytes.len() > request.budgets.max_slice_bytes {
        return Err(budget("flow slice exceeds retained 64 MiB bound"));
    }
    let slice_ref = CasObject::for_bytes(FLOW_SLICE_SCHEMA, &slice_bytes)?;
    let projection = FlowSliceProjection {
        schema: FLOW_SLICE_PROJECTION_SCHEMA.into(),
        flow_id,
        root: request.root.clone(),
        member_alias: request.member_alias.clone(),
        root_node_id,
        counts,
        status,
        certainty,
        order_authority: projection_order_authority,
        verification_obligations: obligations,
    };
    Ok(PreparedFlowSlice {
        slice,
        slice_bytes,
        slice_ref,
        projection,
    })
}

pub fn verify_prepared(
    prepared: &PreparedFlowSlice,
    fact_set: &PreparedCallableFactSet,
) -> Result<(), ClewError> {
    if canonical::bytes(&prepared.slice).map_err(internal)? != prepared.slice_bytes
        || CasObject::for_bytes(FLOW_SLICE_SCHEMA, &prepared.slice_bytes)? != prepared.slice_ref
        || prepared.projection.flow_id != prepared.slice.flow_id
        || prepared.slice.request.fact_set_authority_digest != fact_set.authority.authority_digest
        || prepared.slice.counts.nodes != prepared.slice.nodes.len()
        || prepared.slice.counts.edges != prepared.slice.edges.len()
        || prepared.slice.counts.boundaries != prepared.slice.boundaries.len()
        || prepared.slice.counts.control_flow_regions != prepared.slice.control_flow_regions.len()
    {
        return Err(corrupt("prepared flow slice is internally inconsistent"));
    }
    let rebuilt = build(prepared.slice.request.clone(), fact_set)?;
    if &rebuilt != prepared {
        return Err(corrupt(
            "prepared flow slice differs from deterministic retained facts",
        ));
    }
    Ok(())
}

pub fn verify_prepared_with_cfg(
    prepared: &PreparedFlowSlice,
    fact_set: &PreparedCallableFactSet,
    cfg: &LocalCfgCatalog,
) -> Result<(), ClewError> {
    if canonical::bytes(&prepared.slice).map_err(internal)? != prepared.slice_bytes
        || CasObject::for_bytes(FLOW_SLICE_SCHEMA, &prepared.slice_bytes)? != prepared.slice_ref
    {
        return Err(corrupt(
            "prepared CFG flow slice is internally inconsistent",
        ));
    }
    let rebuilt = build_with_cfg(prepared.slice.request.clone(), fact_set, cfg)?;
    if &rebuilt != prepared {
        return Err(corrupt(
            "prepared CFG flow slice differs from deterministic retained facts",
        ));
    }
    Ok(())
}

fn validate_request(
    request: &FlowRequest,
    fact_set: &PreparedCallableFactSet,
) -> Result<(), ClewError> {
    request.budgets.validate()?;
    crate::semantic_validation::validate_kotlin_full_symbol_identity(&request.root)?;
    if request.thread_id != fact_set.authority.thread_id
        || request.thread_authority_digest != fact_set.authority.thread_authority_digest
        || request.fact_set_authority_digest != fact_set.authority.authority_digest
        || request.fact_set_id != fact_set.projection.fact_set_id
    {
        return Err(invalid(
            "flow request is not bound to its callable fact set",
        ));
    }
    let pair = fact_set
        .authority
        .pairs
        .iter()
        .find(|pair| pair.pair_id == request.pair_id)
        .ok_or_else(|| invalid("flow request names an unknown selected pair"))?;
    if request.member_alias != pair.provider_member && request.member_alias != pair.consumer_member
    {
        return Err(invalid("flow root member is outside the selected pair"));
    }
    Ok(())
}

fn read_facts(fact_set: &PreparedCallableFactSet) -> Result<Vec<IndexedFact>, ClewError> {
    let mut facts = Vec::new();
    for object in &fact_set.fact_shards {
        if CasObject::for_bytes(&object.reference.object_schema, &object.bytes)? != object.reference
        {
            return Err(corrupt(
                "callable fact shard differs from its CAS authority",
            ));
        }
        let shard: CallableFactShard = serde_json::from_slice(&object.bytes)
            .map_err(|_| corrupt("callable fact shard is invalid"))?;
        if canonical::bytes(&shard).map_err(internal)? != object.bytes {
            return Err(corrupt("callable fact shard is not canonical"));
        }
        for fact in shard.facts {
            facts.push(IndexedFact {
                fact,
                shard_ref: object.reference.clone(),
            });
        }
    }
    facts.sort_by(|left, right| left.fact.fact_id().cmp(right.fact.fact_id()));
    Ok(facts)
}

fn callable_node(indexed: &IndexedFact, depth: usize) -> Result<FlowNode, ClewError> {
    let CallableFact::Declaration(row) = &indexed.fact else {
        return Err(corrupt("callable node support is not a declaration"));
    };
    let node_id = stable_id(
        "flow-node",
        &json!({
            "member": row.provenance.member_alias,
            "repository": row.provenance.repository_namespace,
            "symbol": row.symbol_identity,
        }),
    )?;
    Ok(FlowNode {
        node_id,
        node_kind: FlowNodeKind::Callable,
        member_alias: row.provenance.member_alias.clone(),
        repository_namespace: row.provenance.repository_namespace.clone(),
        symbol_identity: row.symbol_identity.clone(),
        depth,
        order_authority: FlowOrderAuthority::UnorderedStaticRelation,
        support_refs: vec![indexed.support()],
    })
}

fn node_for_symbol(nodes: &BTreeMap<String, FlowNode>, symbol: &str) -> Option<String> {
    nodes
        .values()
        .find(|node| node.symbol_identity == symbol)
        .map(|node| node.node_id.clone())
}

fn relation_cfg_nodes(
    row: &crate::thread_callables::UseFact,
    graph: &LocalCfgPayload,
) -> Option<Vec<u64>> {
    if row
        .relation_evidence
        .get("orderProvenance")
        .and_then(serde_json::Value::as_str)
        != Some("K2_FIR_CFG")
    {
        return None;
    }
    let known = graph
        .nodes
        .iter()
        .map(|node| node.node_id)
        .collect::<BTreeSet<_>>();
    let values = row
        .relation_evidence
        .get("cfgNodeIds")?
        .as_array()?
        .iter()
        .map(serde_json::Value::as_u64)
        .collect::<Option<Vec<_>>>()?;
    if values.is_empty()
        || values.windows(2).any(|pair| pair[0] >= pair[1])
        || values.iter().any(|node| !known.contains(node))
    {
        return None;
    }
    Some(values)
}

fn is_zero(value: &usize) -> bool {
    *value == 0
}

#[allow(clippy::too_many_arguments)]
fn add_boundary(
    boundaries: &mut BTreeMap<String, FlowBoundary>,
    obligations: &mut BTreeSet<String>,
    request: &FlowRequest,
    code: &str,
    subject: &str,
    mut required_checks: Vec<String>,
    mut support_refs: Vec<FlowSupportRef>,
) -> Result<(), ClewError> {
    required_checks.sort();
    required_checks.dedup();
    support_refs.sort_by(|left, right| left.fact_id.cmp(&right.fact_id));
    for check in &required_checks {
        obligations.insert(check.clone());
    }
    let boundary_id = stable_id(
        "flow-boundary",
        &json!({
            "member": request.member_alias,
            "code": code,
            "subject": subject,
            "support": support_refs,
        }),
    )?;
    if !boundaries.contains_key(&boundary_id) && boundaries.len() >= request.budgets.max_boundaries
    {
        return Err(budget("flow boundary budget exceeded"));
    }
    boundaries
        .entry(boundary_id.clone())
        .or_insert(FlowBoundary {
            boundary_id,
            code: code.into(),
            subject: subject.into(),
            required_checks,
            support_refs,
        });
    Ok(())
}

fn stable_id(prefix: &str, value: &impl Serialize) -> Result<String, ClewError> {
    Ok(format!(
        "{prefix}:{}",
        digest_component(&canonical::hash(value).map_err(internal)?)?
    ))
}

fn digest_component(value: &str) -> Result<&str, ClewError> {
    value
        .strip_prefix("sha256:")
        .filter(|digest| digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or_else(|| corrupt("canonical digest is invalid"))
}

fn invalid(message: impl Into<String>) -> ClewError {
    ClewError::new(ErrorCode::InvalidInput, message)
}

fn corrupt(message: impl Into<String>) -> ClewError {
    ClewError::new(ErrorCode::StateCorrupt, message)
}

fn budget(message: impl Into<String>) -> ClewError {
    ClewError::new(ErrorCode::SliceBudgetExceeded, message)
}

fn internal(message: impl ToString) -> ClewError {
    ClewError::new(ErrorCode::Internal, message.to_string())
}
