//! Adapter from an executable semantic thread to repository-neutral L0-L5 facts.

use crate::canonical;
use crate::model::{CompletenessStatus, Edge, GraphNode, ThreadIr};
use crate::projection::{
    BoundaryState, L0Source, PROJECTION_SCHEMA, ProjectionError, ProjectionLevel,
    ProjectionProvenance, SemanticBoundary, SemanticEdge, SemanticFact, SemanticProjectionInput,
    ThreadKind,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadProjection {
    pub input: SemanticProjectionInput,
    pub root_fact_id: String,
}

#[derive(Debug, Error)]
pub enum ThreadProjectionError {
    #[error("semantic thread has no anchored L0 evidence")]
    NoAnchoredEvidence,
    #[error("semantic thread has no {0:?} evidence")]
    UnsupportedThreadKind(ThreadKind),
    #[error("semantic thread has no {0} provenance")]
    MissingProvenance(&'static str),
    #[error("semantic graph node {0} has inconsistent source provenance")]
    InvalidSourceProvenance(String),
    #[error("cannot construct projection fact: {0}")]
    Projection(#[from] ProjectionError),
    #[error("cannot canonicalize thread projection: {0}")]
    Canonical(String),
}

/// Builds a generic abstraction ladder from compiler-produced Thread IR.
///
/// The adapter performs no repository lookup and never invents an L5 claim:
/// L5 exists only when the caller supplies explicit intent. L1-L4 summaries
/// contain structural counts and resolved identifiers, never source snippets.
pub fn from_thread(
    thread: &ThreadIr,
    thread_kind: ThreadKind,
    claim: Option<&str>,
) -> Result<ThreadProjection, ThreadProjectionError> {
    if !available_thread_kinds(thread).contains(&thread_kind) {
        return Err(ThreadProjectionError::UnsupportedThreadKind(thread_kind));
    }
    let provenance = projection_provenance(thread)?;
    let mut facts = Vec::new();
    let mut l0_by_graph_node = BTreeMap::new();

    for node in &thread.nodes {
        let source = source_provenance(node)?;
        let id = stable_id("l0", &json!({"thread":thread.thread_id,"node":node.id}))?;
        let semantic_label = node
            .attributes
            .get("symbol")
            .or_else(|| node.attributes.get("type"))
            .and_then(Value::as_str)
            .unwrap_or("resolved");
        facts.push(SemanticFact::new(
            id.clone(),
            ProjectionLevel::L0,
            node.kind.clone(),
            format!("{} {semantic_label}", node.kind),
            vec![],
            Some(source),
        )?);
        l0_by_graph_node.insert(node.id.clone(), id);
    }
    if facts.is_empty() {
        return Err(ThreadProjectionError::NoAnchoredEvidence);
    }
    let mut evidence: Vec<_> = l0_by_graph_node.values().cloned().collect();
    evidence.sort();
    evidence.dedup();

    let owner = thread
        .seed
        .get("symbol")
        .and_then(Value::as_str)
        .or_else(|| {
            thread
                .nodes
                .iter()
                .filter_map(|node| node.origin.as_ref())
                .find_map(|origin| origin.get("ownerSymbolId").and_then(Value::as_str))
        })
        .unwrap_or("<semantic-root>");
    let l1 = stable_id("l1", &json!({"thread":thread.thread_id,"owner":owner}))?;
    let l2 = stable_id("l2", &json!({"thread":thread.thread_id,"owner":owner}))?;
    let l3 = stable_id(
        "l3",
        &json!({"thread":thread.thread_id,"kind":thread_kind,"owner":owner}),
    )?;
    let l4 = stable_id(
        "l4",
        &json!({"thread":thread.thread_id,"compilation":thread.snapshot.compilation,"owner":owner}),
    )?;

    facts.push(SemanticFact::new(
        l1.clone(),
        ProjectionLevel::L1,
        "SYMBOL",
        format!("resolved symbol {owner}"),
        evidence.clone(),
        None,
    )?);
    facts.push(SemanticFact::new(
        l2.clone(),
        ProjectionLevel::L2,
        "COMPONENT_CONTRACT_EFFECT",
        format!(
            "component behavior with {} semantic nodes and {} dependency edges",
            thread.nodes.len(),
            thread.edges.len()
        ),
        evidence.clone(),
        None,
    )?);
    facts.push(SemanticFact::new(
        l3.clone(),
        ProjectionLevel::L3,
        "SEMANTIC_THREAD",
        format!("{thread_kind:?} thread for {owner}"),
        evidence.clone(),
        None,
    )?);
    facts.push(SemanticFact::new(
        l4.clone(),
        ProjectionLevel::L4,
        "ARCHITECTURE_OWNERSHIP",
        format!("compilation {} owns {owner}", thread.snapshot.compilation),
        evidence.clone(),
        None,
    )?);

    let mut edges = graph_edges(&thread.edges, &l0_by_graph_node)?;
    for l0 in &evidence {
        edges.push(hierarchy_edge(&l1, l0, thread_kind)?);
    }
    edges.push(hierarchy_edge(&l2, &l1, thread_kind)?);
    edges.push(hierarchy_edge(&l3, &l2, thread_kind)?);
    edges.push(hierarchy_edge(&l4, &l3, thread_kind)?);

    let root_fact_id = if let Some(claim) = claim.filter(|claim| !claim.trim().is_empty()) {
        let l5 = stable_id(
            "l5",
            &json!({"thread":thread.thread_id,"kind":thread_kind,"claim":claim}),
        )?;
        facts.push(SemanticFact::new(
            l5.clone(),
            ProjectionLevel::L5,
            "EXPLICIT_CLAIM",
            claim,
            evidence.clone(),
            None,
        )?);
        edges.push(hierarchy_edge(&l5, &l4, thread_kind)?);
        l5
    } else {
        l4.clone()
    };

    facts.sort_by(|left, right| left.id.cmp(&right.id));
    edges.sort_by(|left, right| left.id.cmp(&right.id));
    edges.dedup_by(|left, right| left.id == right.id);
    let affected_fact_ids = facts.iter().map(|fact| fact.id.clone()).collect::<Vec<_>>();
    let boundaries = thread_boundary(thread, affected_fact_ids);

    Ok(ThreadProjection {
        input: SemanticProjectionInput {
            schema: PROJECTION_SCHEMA.to_owned(),
            provenance,
            facts,
            edges,
            boundaries,
        },
        root_fact_id,
    })
}

fn projection_provenance(thread: &ThreadIr) -> Result<ProjectionProvenance, ThreadProjectionError> {
    let classpath_hash = read_hash(thread, "CLASSPATH", "classpath")?;
    let compiler_options_hash = read_hash(thread, "COMPILER_OPTIONS", "compiler options")?;
    let composite_snapshot_hash = canonical::hash(&json!({
        "baseRevision":thread.snapshot.base_revision,
        "indexSnapshot":thread.snapshot.index_snapshot,
        "projectModelHash":thread.snapshot.project_model_hash,
        "classpathHash":classpath_hash,
        "compilerVersion":thread.snapshot.compiler_version,
        "compilerOptionsHash":compiler_options_hash,
        "compilation":thread.snapshot.compilation,
    }))
    .map_err(|error| ThreadProjectionError::Canonical(error.to_string()))?;
    Ok(ProjectionProvenance {
        base_revision: thread.snapshot.base_revision.clone(),
        composite_snapshot_hash,
        index_snapshot_hash: thread.snapshot.index_snapshot.clone(),
        project_model_hash: thread.snapshot.project_model_hash.clone(),
        classpath_hash,
        compiler_version: thread.snapshot.compiler_version.clone(),
        compiler_options_hash,
        compilation: thread.snapshot.compilation.clone(),
    })
}

fn read_hash(
    thread: &ThreadIr,
    kind: &str,
    label: &'static str,
) -> Result<String, ThreadProjectionError> {
    thread
        .read_set
        .iter()
        .find(|fact| fact.kind == kind)
        .map(|fact| fact.hash.clone())
        .filter(|hash| !hash.is_empty())
        .ok_or(ThreadProjectionError::MissingProvenance(label))
}

fn source_provenance(node: &GraphNode) -> Result<L0Source, ThreadProjectionError> {
    let invalid = || ThreadProjectionError::InvalidSourceProvenance(node.id.clone());
    let origin = node.origin.as_ref().ok_or_else(|| invalid())?;
    let range = origin
        .get("rangeHint")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid())?;
    let range_start = range
        .first()
        .and_then(Value::as_u64)
        .ok_or_else(|| invalid())?;
    let range_end = range
        .get(1)
        .and_then(Value::as_u64)
        .ok_or_else(|| invalid())?;
    let file = origin
        .get("file")
        .or_else(|| origin.get("fileId"))
        .and_then(Value::as_str)
        .ok_or_else(|| invalid())?
        .to_owned();
    let content_hash = origin
        .get("exactTextHash")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid())?
        .to_owned();
    let snippet = origin
        .get("sourceText")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| invalid())?;
    if canonical::hash_bytes(snippet.as_bytes()) != content_hash {
        return Err(invalid());
    }
    if file.is_empty() || range_end <= range_start {
        return Err(invalid());
    }
    Ok(L0Source {
        file,
        content_hash,
        range_start,
        range_end,
        snippet: Some(snippet),
    })
}

/// Returns only vertical thread kinds evidenced by compiler-produced graph
/// facts or by the Thread IR contour itself. A caller cannot relabel a graph.
pub fn available_thread_kinds(thread: &ThreadIr) -> BTreeSet<ThreadKind> {
    let mut kinds = BTreeSet::new();
    let node_ids: BTreeSet<_> = thread.nodes.iter().map(|node| node.id.as_str()).collect();
    let seed_node_id = thread.seed.get("nodeId").and_then(Value::as_str);
    if seed_node_id.is_some_and(|seed| {
        node_ids.contains(seed)
            && thread.edges.iter().any(|edge| {
                (edge.from == seed || edge.to == seed)
                    && edge_kind(&edge.kind).is_some()
                    && node_ids.contains(edge.from.as_str())
                    && node_ids.contains(edge.to.as_str())
            })
    }) {
        kinds.insert(ThreadKind::Journey);
    }
    for edge in &thread.edges {
        if let Some(kind) = edge_kind(&edge.kind) {
            kinds.insert(kind);
        }
    }
    if thread
        .nodes
        .iter()
        .any(|node| node.defines.is_some() || !node.uses.is_empty())
    {
        kinds.insert(ThreadKind::Data);
    }
    for node in &thread.nodes {
        match node.kind.as_str() {
            "BRANCH" | "LOOP" => {
                kinds.insert(ThreadKind::Control);
            }
            "CALL" | "CALL_RESULT" | "RETURN" | "EXPRESSION" | "DEFINITION" | "ASSIGNMENT"
            | "PHI" => {
                kinds.insert(ThreadKind::Data);
            }
            "THROW" => {
                kinds.insert(ThreadKind::Failure);
            }
            "CONFIG_READ" | "CONFIG_WRITE" => {
                kinds.insert(ThreadKind::Config);
            }
            _ => {}
        }
        for effect in node
            .attributes
            .get("effects")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            match effect {
                "READ_STATE" | "WRITE_STATE" => {
                    kinds.insert(ThreadKind::State);
                }
                "IO" | "SUSPEND" => {
                    kinds.insert(ThreadKind::Effect);
                }
                "THROW" => {
                    kinds.insert(ThreadKind::Failure);
                }
                _ => {}
            }
        }
    }
    if is_test_compilation(&thread.snapshot.compilation) {
        kinds.insert(ThreadKind::TestEvidence);
    }
    if thread.seed.get("kind").and_then(Value::as_str) == Some("CHANGE")
        && seed_node_id.is_some_and(|seed| node_ids.contains(seed))
    {
        kinds.insert(ThreadKind::Change);
    }
    kinds
}

fn is_test_compilation(compilation: &str) -> bool {
    compilation.ends_with("/test") || compilation.ends_with("/testFixtures")
}

fn graph_edges(
    graph_edges: &[Edge],
    l0_by_graph_node: &BTreeMap<String, String>,
) -> Result<Vec<SemanticEdge>, ThreadProjectionError> {
    graph_edges
        .iter()
        .filter_map(|edge| {
            Some((
                l0_by_graph_node.get(&edge.from)?.clone(),
                l0_by_graph_node.get(&edge.to)?.clone(),
                edge_kind(&edge.kind)?,
            ))
        })
        .map(|(from, to, kind)| {
            Ok(SemanticEdge {
                id: stable_id("edge", &json!({"from":from,"to":to,"kind":kind}))?,
                from,
                to,
                kind,
            })
        })
        .collect()
}

fn hierarchy_edge(
    from: &str,
    to: &str,
    kind: ThreadKind,
) -> Result<SemanticEdge, ThreadProjectionError> {
    Ok(SemanticEdge {
        id: stable_id("edge", &json!({"from":from,"to":to,"kind":kind}))?,
        from: from.to_owned(),
        to: to.to_owned(),
        kind,
    })
}

fn edge_kind(kind: &str) -> Option<ThreadKind> {
    match kind {
        "CONTROL_DEP" | "TRUE" | "FALSE" | "CFG_TRUE" | "CFG_FALSE" => Some(ThreadKind::Control),
        "READ_STATE" | "WRITE_STATE" => Some(ThreadKind::State),
        "THROW" | "CFG_EXCEPTION" => Some(ThreadKind::Failure),
        "IO" | "SUSPEND" => Some(ThreadKind::Effect),
        "CONFIG" | "CONFIG_READ" | "CONFIG_WRITE" => Some(ThreadKind::Config),
        "TEST_EVIDENCE" => Some(ThreadKind::TestEvidence),
        "CHANGE" => Some(ThreadKind::Change),
        "DEF_USE" | "PHI_INPUT" | "CALL" | "RETURN" | "CFG_NORMAL" | "FLOW" | "ARGUMENT"
        | "RECEIVER" => Some(ThreadKind::Data),
        _ => None,
    }
}

fn thread_boundary(thread: &ThreadIr, affected_fact_ids: Vec<String>) -> Vec<SemanticBoundary> {
    let state = match thread.completeness.status {
        CompletenessStatus::CompleteSupportedSubset => return vec![],
        CompletenessStatus::PartialBudget => BoundaryState::Partial,
        CompletenessStatus::PartialUnsupportedFeature
        | CompletenessStatus::PartialExternalBoundary
        | CompletenessStatus::PartialDynamicDispatch => BoundaryState::Unsupported,
        CompletenessStatus::Failed => BoundaryState::Unknown,
    };
    vec![SemanticBoundary {
        id: "THREAD_COMPLETENESS".to_owned(),
        state,
        affected_fact_ids,
        reason: canonical::hash(&thread.completeness.boundaries)
            .unwrap_or_else(|_| "UNAVAILABLE".to_owned()),
    }]
}

fn stable_id(prefix: &str, value: &Value) -> Result<String, ThreadProjectionError> {
    canonical::hash(value)
        .map(|hash| format!("{prefix}:{}", hash.trim_start_matches("sha256:")))
        .map_err(|error| ThreadProjectionError::Canonical(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Completeness, Direction, GraphNode, ReadFact, SlicePolicy, Snapshot};
    use crate::projection::{
        BoundaryPolicy, ProjectionBudget, ProjectionQuery, ProjectionStatus, Traversal, project,
        validate_trace_to_l0,
    };

    fn thread(status: CompletenessStatus) -> ThreadIr {
        let origin = |id: &str, start: u64| {
            json!({
                "fileId":"src/main/kotlin/A.kt",
                "ownerSymbolId":"p.A.run",
                "exactTextHash":canonical::hash_bytes(id.as_bytes()),
                "rangeHint":[start,start + 4],
                "sourceText":id
            })
        };
        ThreadIr {
            schema: "semantic-thread/0.1".into(),
            thread_id: "thread:one".into(),
            snapshot: Snapshot {
                base_revision: "revision".into(),
                project_model_hash: "model".into(),
                compiler_version: "2.1.21".into(),
                index_snapshot: "index".into(),
                compilation: ":/main".into(),
                ..Snapshot::default()
            },
            seed: json!({"symbol":"p.A.run"}),
            policy: SlicePolicy {
                direction: Direction::Both,
                ..SlicePolicy::default()
            },
            completeness: Completeness {
                status,
                boundaries: vec![json!({"kind":"EXTERNAL_CALL"})],
            },
            nodes: vec![
                GraphNode {
                    id: "n1".into(),
                    kind: "CALL".into(),
                    defines: None,
                    uses: vec![],
                    origin: Some(origin("call", 1)),
                    editable: true,
                    attributes: BTreeMap::from([("symbol".into(), json!("p.Service.load"))]),
                },
                GraphNode {
                    id: "n2".into(),
                    kind: "RETURN".into(),
                    defines: None,
                    uses: vec![],
                    origin: Some(origin("return", 10)),
                    editable: true,
                    attributes: BTreeMap::new(),
                },
            ],
            edges: vec![Edge {
                from: "n1".into(),
                to: "n2".into(),
                kind: "DEF_USE".into(),
            }],
            editable_units: vec![],
            external_summaries: vec![],
            read_set: vec![
                ReadFact {
                    kind: "CLASSPATH".into(),
                    key: "p.A.run".into(),
                    hash: "classpath".into(),
                },
                ReadFact {
                    kind: "COMPILER_OPTIONS".into(),
                    key: "p.A.run".into(),
                    hash: "options".into(),
                },
            ],
            validation_plan: vec![],
        }
    }

    #[test]
    fn thread_builds_a_traceable_l5_to_l0_ladder() {
        let projection = from_thread(
            &thread(CompletenessStatus::CompleteSupportedSubset),
            ThreadKind::Data,
            Some("change the load journey"),
        )
        .unwrap();
        let result = project(
            &projection.input,
            &ProjectionQuery {
                schema: PROJECTION_SCHEMA.into(),
                level: ProjectionLevel::L5,
                roots: vec![projection.root_fact_id],
                thread_kinds: vec![ThreadKind::Data],
                traversal: Traversal::Both,
                budget: ProjectionBudget::default(),
                boundary_policy: BoundaryPolicy::ReturnPartial,
            },
        )
        .unwrap();
        assert_eq!(result.status, ProjectionStatus::Complete);
        assert_eq!(result.nodes.len(), 1);
        assert!(result.nodes[0].source.is_none());
        assert_eq!(result.nodes[0].evidence.len(), 2);
        assert_eq!(
            projection
                .input
                .facts
                .iter()
                .find(|fact| fact.level == ProjectionLevel::L2)
                .unwrap()
                .kind,
            "COMPONENT_CONTRACT_EFFECT"
        );
        assert_eq!(
            projection
                .input
                .facts
                .iter()
                .find(|fact| fact.level == ProjectionLevel::L3)
                .unwrap()
                .kind,
            "SEMANTIC_THREAD"
        );
        assert_eq!(
            projection
                .input
                .facts
                .iter()
                .find(|fact| fact.level == ProjectionLevel::L4)
                .unwrap()
                .kind,
            "ARCHITECTURE_OWNERSHIP"
        );
        assert!(
            result.nodes[0]
                .evidence
                .iter()
                .all(|link| link.path.len() == 6)
        );
        validate_trace_to_l0(&projection.input, &result).unwrap();
    }

    #[test]
    fn incomplete_thread_remains_an_explicit_projection_boundary() {
        let projection = from_thread(
            &thread(CompletenessStatus::PartialExternalBoundary),
            ThreadKind::Data,
            None,
        )
        .unwrap();
        let result = project(
            &projection.input,
            &ProjectionQuery {
                schema: PROJECTION_SCHEMA.into(),
                level: ProjectionLevel::L4,
                roots: vec![projection.root_fact_id],
                thread_kinds: vec![ThreadKind::Data],
                traversal: Traversal::Both,
                budget: ProjectionBudget::default(),
                boundary_policy: BoundaryPolicy::ReturnPartial,
            },
        )
        .unwrap();
        assert_eq!(result.status, ProjectionStatus::PartialBoundary);
    }

    #[test]
    fn caller_cannot_relabel_a_data_thread_as_config() {
        let thread = thread(CompletenessStatus::CompleteSupportedSubset);
        assert!(matches!(
            from_thread(&thread, ThreadKind::Config, None),
            Err(ThreadProjectionError::UnsupportedThreadKind(
                ThreadKind::Config
            ))
        ));
    }

    #[test]
    fn all_vertical_kinds_require_generic_semantic_evidence() {
        let mut thread = thread(CompletenessStatus::CompleteSupportedSubset);
        thread.nodes[0].kind = "BRANCH".to_owned();
        thread.nodes[0]
            .attributes
            .insert("effects".into(), json!(["READ_STATE", "IO"]));
        thread.nodes[1].kind = "THROW".to_owned();
        thread.nodes.push(GraphNode {
            id: "config".into(),
            kind: "CONFIG_READ".into(),
            defines: None,
            uses: vec![],
            origin: None,
            editable: false,
            attributes: BTreeMap::new(),
        });
        thread.snapshot.compilation = ":/test".into();
        thread.seed["kind"] = json!("CHANGE");
        thread.seed["nodeId"] = json!("n1");
        let kinds = available_thread_kinds(&thread);
        assert_eq!(
            kinds,
            BTreeSet::from([
                ThreadKind::Control,
                ThreadKind::Data,
                ThreadKind::Journey,
                ThreadKind::State,
                ThreadKind::Effect,
                ThreadKind::Failure,
                ThreadKind::Config,
                ThreadKind::TestEvidence,
                ThreadKind::Change,
            ])
        );
    }

    #[test]
    fn decoy_attribute_names_do_not_create_thread_kinds() {
        let mut thread = thread(CompletenessStatus::CompleteSupportedSubset);
        thread.snapshot.compilation = ":/contest".into();
        thread.nodes[0].attributes.extend([
            ("config".into(), json!(true)),
            ("test".into(), json!(true)),
            ("effect".into(), json!("IO")),
            ("failure".into(), json!("THROW")),
        ]);
        let kinds = available_thread_kinds(&thread);
        assert!(!kinds.contains(&ThreadKind::Config));
        assert!(!kinds.contains(&ThreadKind::TestEvidence));
        assert!(!kinds.contains(&ThreadKind::Effect));
        assert!(!kinds.contains(&ThreadKind::Failure));
    }

    #[test]
    fn unanchored_or_non_exact_source_nodes_fail_closed() {
        let mut unanchored = thread(CompletenessStatus::CompleteSupportedSubset);
        unanchored.nodes[1].origin = None;
        assert!(matches!(
            from_thread(&unanchored, ThreadKind::Data, None),
            Err(ThreadProjectionError::InvalidSourceProvenance(id)) if id == "n2"
        ));

        let mut token_only = thread(CompletenessStatus::CompleteSupportedSubset);
        let origin = token_only.nodes[1].origin.as_mut().unwrap();
        origin["normalizedTokenHash"] = origin["exactTextHash"].clone();
        origin.as_object_mut().unwrap().remove("exactTextHash");
        assert!(matches!(
            from_thread(&token_only, ThreadKind::Data, None),
            Err(ThreadProjectionError::InvalidSourceProvenance(id)) if id == "n2"
        ));

        let mut no_source = thread(CompletenessStatus::CompleteSupportedSubset);
        no_source.nodes[1]
            .origin
            .as_mut()
            .unwrap()
            .as_object_mut()
            .unwrap()
            .remove("sourceText");
        assert!(matches!(
            from_thread(&no_source, ThreadKind::Data, None),
            Err(ThreadProjectionError::InvalidSourceProvenance(id)) if id == "n2"
        ));
    }

    #[test]
    fn journey_requires_a_real_seed_edge() {
        let mut isolated = thread(CompletenessStatus::CompleteSupportedSubset);
        isolated.seed["nodeId"] = json!("n1");
        isolated.edges.clear();
        assert!(!available_thread_kinds(&isolated).contains(&ThreadKind::Journey));

        let mut dangling = thread(CompletenessStatus::CompleteSupportedSubset);
        dangling.seed["nodeId"] = json!("n1");
        dangling.edges[0].to = "missing".into();
        assert!(!available_thread_kinds(&dangling).contains(&ThreadKind::Journey));

        let mut connected = thread(CompletenessStatus::CompleteSupportedSubset);
        connected.seed["nodeId"] = json!("n1");
        assert!(available_thread_kinds(&connected).contains(&ThreadKind::Journey));
    }
}
