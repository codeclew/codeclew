//! One-hop, single-repository consequences. Relation evidence and affected
//! candidates are separate claims; no runtime failure or test coverage is inferred.
use crate::canonical;
use crate::cas::CasObject;
use crate::error::{ClewError, ErrorCode};
use crate::working_tree_change::{Analysis, Comparison, Declaration, SourceAnchor};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_RELATIONS: usize = 32_768;
const MAX_NODES: usize = 512;
const MAX_EDGES: usize = 2_048;
const MAX_SUPPORT_PER_SIDE: usize = 64;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Relation {
    pub compilation: String,
    pub owner: String,
    pub target: String,
    pub kind: String,
    pub source: SourceAnchor,
    pub fact_key: String,
    pub payload: CasObject,
    pub exact_call_target: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Node {
    pub node_id: String,
    pub role: String,
    pub before: Option<Declaration>,
    pub after: Option<Declaration>,
    pub change_id: Option<String>,
    pub entrypoints: Vec<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Edge {
    pub edge_id: String,
    pub owner_node: String,
    pub target_node: String,
    pub kind: String,
    pub presence: String,
    pub authority: String,
    pub before: Vec<Relation>,
    pub after: Vec<Relation>,
    pub before_occurrences: usize,
    pub after_occurrences: usize,
    pub omitted_support_count: usize,
    pub claim_freshness: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Candidate {
    pub node_id: String,
    pub changed_node_id: String,
    pub edge_id: String,
    pub reason: String,
    pub authority: String,
    pub selected_test_compilation: bool,
    pub compilation: String,
    pub source: SourceAnchor,
    pub verification: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Consequences {
    pub schema: String,
    pub scope: String,
    pub status: String,
    pub before_snapshot: CasObject,
    pub after_snapshot: CasObject,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub candidates: Vec<Candidate>,
    pub boundaries: Vec<Value>,
    pub omitted_node_count: usize,
    pub omitted_edge_count: usize,
    pub unresolved_relation_count: usize,
    pub test_scope: String,
    pub tests_executed: bool,
    pub obligations: Vec<String>,
}

fn declaration_key(d: &Declaration) -> (String, String) {
    (d.compilation.clone(), d.symbol.clone())
}
fn node_id(d: &Declaration) -> Result<String, ClewError> {
    hash(&json!([d.compilation, d.symbol, d.source.file]))
}
fn hash(value: &impl Serialize) -> Result<String, ClewError> {
    canonical::hash(value).map_err(|error| ClewError::new(ErrorCode::Internal, error.to_string()))
}

struct Lookup<'a> {
    exact: BTreeMap<(String, String), &'a Declaration>,
    families: BTreeMap<(String, String), Vec<&'a Declaration>>,
    symbols: BTreeMap<String, Vec<&'a Declaration>>,
}
impl<'a> Lookup<'a> {
    fn new(analysis: &'a Analysis) -> Self {
        let mut lookup = Self {
            exact: BTreeMap::new(),
            families: BTreeMap::new(),
            symbols: BTreeMap::new(),
        };
        for d in &analysis.declarations {
            lookup.exact.insert(declaration_key(d), d);
            lookup.symbols.entry(d.symbol.clone()).or_default().push(d);
            if let Some(family) = &d.family {
                lookup
                    .families
                    .entry((d.compilation.clone(), family.clone()))
                    .or_default()
                    .push(d);
            }
        }
        lookup
    }
    fn owner(&self, relation: &Relation) -> Option<&'a Declaration> {
        let contained = |d: &&Declaration| {
            d.source.file == relation.source.file
                && d.source.start <= relation.source.start
                && d.source.end >= relation.source.end
        };
        if let Some(d) = self
            .exact
            .get(&(relation.compilation.clone(), relation.owner.clone()))
        {
            return contained(d).then_some(*d);
        }
        let candidates = self
            .families
            .get(&(relation.compilation.clone(), relation.owner.clone()))?;
        let mut matches = candidates.iter().copied().filter(contained);
        let first = matches.next()?;
        matches.next().is_none().then_some(first)
    }
    fn target(&self, relation: &Relation) -> Option<&'a Declaration> {
        // A bare callable family cannot choose among current/future overloads.
        if matches!(relation.kind.as_str(), "CALLS" | "CONSTRUCTS") && !relation.exact_call_target {
            return None;
        }
        if let Some(d) = self
            .exact
            .get(&(relation.compilation.clone(), relation.target.clone()))
        {
            return Some(*d);
        }
        // A selected test compilation can reference a declaration from selected
        // main evidence. Require an exact and unique symbol across that scope.
        let candidates = self.symbols.get(&relation.target)?;
        (candidates.len() == 1).then_some(candidates[0])
    }
}

pub fn build(report: &Comparison) -> Result<Consequences, ClewError> {
    let mut nodes = BTreeMap::<String, Node>::new();
    let mut before_ids = BTreeMap::new();
    let mut after_ids = BTreeMap::new();
    let mut roots = BTreeSet::new();
    for change in &report.declarations {
        let id = format!("change:{}", change.change_id);
        roots.insert(id.clone());
        if let Some(d) = &change.before {
            before_ids.insert(declaration_key(d), id.clone());
        }
        if let Some(d) = &change.after {
            after_ids.insert(declaration_key(d), id.clone());
        }
        nodes.insert(
            id.clone(),
            Node {
                node_id: id,
                role: "CHANGED_DECLARATION".into(),
                before: change.before.clone(),
                after: change.after.clone(),
                change_id: Some(change.change_id.clone()),
                entrypoints: vec![],
            },
        );
    }
    // Bind unchanged neighboring declarations to both exact identities. Their
    // source/shape status is local evidence, never a universal no-impact claim.
    for (analysis, ids, is_before) in [
        (&report.before.analysis, &mut before_ids, true),
        (&report.after.analysis, &mut after_ids, false),
    ] {
        for d in &analysis.declarations {
            if ids.contains_key(&declaration_key(d)) {
                continue;
            }
            let id = node_id(d)?;
            ids.insert(declaration_key(d), id.clone());
            let node = nodes.entry(id.clone()).or_insert_with(|| Node {
                node_id: id,
                role: "OBSERVED_NEIGHBOR".into(),
                before: None,
                after: None,
                change_id: None,
                entrypoints: vec![],
            });
            if is_before {
                node.before = Some(d.clone());
            } else {
                node.after = Some(d.clone());
            }
        }
    }
    let mut edges = BTreeMap::<(String, String, String), Edge>::new();
    let mut boundaries = Vec::new();
    let mut unresolved = 0;
    for (side, analysis, ids) in [
        ("BEFORE", &report.before.analysis, &before_ids),
        ("AFTER", &report.after.analysis, &after_ids),
    ] {
        let lookup = Lookup::new(analysis);
        for relation in &analysis.relations {
            let owner = lookup.owner(relation);
            let target = lookup.target(relation);
            let owner_id = owner.and_then(|d| ids.get(&declaration_key(d)));
            let target_id = target.and_then(|d| ids.get(&declaration_key(d)));
            let near_change = owner_id.is_some_and(|id| roots.contains(id))
                || target_id.is_some_and(|id| roots.contains(id));
            if owner_id.is_none() || target_id.is_none() {
                unresolved += 1;
                if boundaries.len() < 128 {
                    boundaries.push(json!({"side":side,"code":"RELATION_ENDPOINT_UNRESOLVED_IN_SELECTED_SCOPE","rootProximity":if near_change {"DIRECT"} else {"NOT_ESTABLISHED"},"ownerResolved":owner_id.is_some(),"targetResolved":target_id.is_some(),"exactCallTarget":relation.exact_call_target,"kind":relation.kind,"owner":relation.owner,"target":relation.target,"source":relation.source,"payload":relation.payload}));
                }
                continue;
            }
            if !near_change {
                continue;
            }
            let owner_id = owner_id.unwrap();
            let target_id = target_id.unwrap();
            let key = (owner_id.clone(), target_id.clone(), relation.kind.clone());
            let id = hash(&key)?;
            let edge = edges.entry(key).or_insert_with(|| Edge {
                edge_id: id,
                owner_node: owner_id.clone(),
                target_node: target_id.clone(),
                kind: relation.kind.clone(),
                presence: String::new(),
                authority: "COMPILER_RESOLVED_DIRECT_RELATION".into(),
                before: vec![],
                after: vec![],
                before_occurrences: 0,
                after_occurrences: 0,
                omitted_support_count: 0,
                claim_freshness: String::new(),
            });
            let support = if side == "BEFORE" {
                edge.before_occurrences += 1;
                &mut edge.before
            } else {
                edge.after_occurrences += 1;
                &mut edge.after
            };
            if support.len() < MAX_SUPPORT_PER_SIDE {
                support.push(relation.clone());
            } else {
                edge.omitted_support_count += 1;
            }
        }
    }
    let mut selected = roots.clone();
    for edge in edges.values() {
        selected.insert(edge.owner_node.clone());
        selected.insert(edge.target_node.clone());
    }
    let total_nodes = selected.len();
    let mut kept = roots
        .iter()
        .take(MAX_NODES)
        .cloned()
        .collect::<BTreeSet<_>>();
    for id in selected {
        if kept.len() == MAX_NODES {
            break;
        }
        kept.insert(id);
    }
    let total_edges = edges.len();
    let mut edges = edges
        .into_values()
        .filter(|e| kept.contains(&e.owner_node) && kept.contains(&e.target_node))
        .take(MAX_EDGES)
        .collect::<Vec<_>>();
    let mut candidates = Vec::new();
    for edge in &mut edges {
        edge.presence = if edge.after_occurrences == 0 {
            "BEFORE_ONLY"
        } else if edge.before_occurrences == 0 {
            "AFTER_ONLY"
        } else {
            "BOTH_SNAPSHOTS"
        }
        .into();
        edge.claim_freshness = if edge.after_occurrences == 0
            && report.after.analysis.status != "AVAILABLE"
        {
            "UNRESOLVED_AFTER_ANALYSIS"
        } else if edge.after_occurrences == 0 && !report.after.analysis.relation_coverage_complete {
            "NOT_OBSERVED_AFTER_WITH_PARTIAL_COVERAGE"
        } else if edge.after_occurrences == 0 {
            "BEFORE_RELATION_CLAIM_STALE_FOR_AFTER"
        } else if edge.before_occurrences > 0 {
            "DIRECT_RELATION_PRESERVED_NOT_BEHAVIORAL_EQUIVALENCE"
        } else {
            "NEWLY_OBSERVED_DIRECT_RELATION"
        }
        .into();
        if roots.contains(&edge.target_node) && edge.owner_node != edge.target_node {
            let owner = &nodes[&edge.owner_node];
            let d = owner.after.as_ref().or(owner.before.as_ref()).unwrap();
            candidates.push(Candidate {
                node_id: edge.owner_node.clone(),
                changed_node_id: edge.target_node.clone(),
                edge_id: edge.edge_id.clone(),
                reason: "DIRECT_CONSUMER_OF_CHANGED_DECLARATION_IN_BEFORE_OR_AFTER".into(),
                authority: "STATIC_DERIVED_AFFECTED_CANDIDATE".into(),
                selected_test_compilation: d.compilation.ends_with("/test"),
                compilation: d.compilation.clone(),
                source: d.source.clone(),
                verification:
                    "VERIFY_CHANGED_CONTRACT_AND_RELEVANT_TESTS; NO_RUNTIME_FAILURE_PROVEN".into(),
            });
        }
    }
    let mut retained_nodes = nodes
        .into_values()
        .filter(|n| kept.contains(&n.node_id))
        .collect::<Vec<_>>();
    for node in &mut retained_nodes {
        if node.role == "OBSERVED_NEIGHBOR" && node.before.is_some() && node.after.is_some() {
            node.role = "UNCHANGED_SOURCE_AND_PROJECTED_SHAPE".into();
        }
        for (side, d) in [
            ("BEFORE", node.before.as_ref()),
            ("AFTER", node.after.as_ref()),
        ] {
            let Some(d) = d else {
                continue;
            };
            if let Some(entries) = d
                .projected_shape
                .pointer("/spring/entries")
                .and_then(Value::as_array)
            {
                for entry in entries {
                    node.entrypoints.push(json!({"side":side,"authority":"K2_RESOLVED_SPRING_ANNOTATIONS","entry":entry,"payload":d.payload,"source":d.source}));
                }
            }
            if d.kind == "FUNCTION"
                && d.family
                    .as_ref()
                    .is_some_and(|f| f.rsplit('/').next() == Some("main"))
                && d.projected_shape["ownerIdentity"]
                    .as_str()
                    .is_some_and(|o| o.starts_with("package:"))
                && d.projected_shape["effectiveVisibility"] == "public"
                && matches!(
                    d.projected_shape["jvmDescriptor"].as_str(),
                    Some("()V" | "([Ljava/lang/String;)V")
                )
            {
                node.entrypoints.push(json!({"side":side,"authority":"COMPILER_PROJECTED_JVM_MAIN_SIGNATURE","payload":d.payload,"source":d.source}));
            }
        }
    }
    let kotlin = report.after.analysis.authority == "KOTLIN_COMPILER_SUPPORTED_SUBSET";
    let selected_tests = report
        .after
        .session
        .compilations
        .iter()
        .any(|c| c.ends_with("/test"));
    let complete = report.before.analysis.relation_coverage_complete
        && report.after.analysis.relation_coverage_complete;
    Ok(Consequences {
        schema: "codeclew-working-tree-consequences/1.0".into(),
        scope: "DIRECT_RELATIONS_IN_SELECTED_COMPILATIONS_BEFORE_UNION_AFTER".into(),
        status: if !kotlin {
            "SYNTAX_ONLY_NO_RESOLVED_CONSEQUENCES"
        } else if complete {
            "BOUNDED_DIRECT_CANDIDATES"
        } else {
            "PARTIAL_DIRECT_CANDIDATES"
        }
        .into(),
        before_snapshot: report.before.snapshot.clone(),
        after_snapshot: report.after.snapshot.clone(),
        omitted_node_count: total_nodes.saturating_sub(retained_nodes.len()),
        omitted_edge_count: total_edges.saturating_sub(edges.len()),
        unresolved_relation_count: unresolved,
        nodes: retained_nodes,
        edges,
        candidates,
        boundaries,
        test_scope: if selected_tests && kotlin {
            "EXPLICIT_GRADLE_TEST_COMPILATION_SELECTED_RELATION_EVIDENCE_ONLY"
        } else {
            "TEST_COMPILATION_NOT_ANALYZED"
        }
        .into(),
        tests_executed: false,
        obligations: vec![
            "DIRECT_CONSUMERS_ARE_CANDIDATES_NOT_RUNTIME_FAILURES".into(),
            "DISPATCH_REFLECTION_FRAMEWORK_AND_EXTERNAL_CONSUMERS_ARE_NOT_CLOSED".into(),
            "RELATION_COUNTS_DO_NOT_ESTABLISH_EXECUTION_ORDER_OR_FREQUENCY".into(),
            "NO_TEST_SUFFICIENCY_OR_EXECUTION_IS_PROVEN".into(),
            "NO_GLOBAL_NO_IMPACT_VERDICT".into(),
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::RuntimeMode;
    use crate::session::{ModelCachePolicy, SessionAuthority, SessionLanguage};
    use crate::working_tree_change::{DeclarationChange, Side};

    fn object() -> CasObject {
        serde_json::from_value(json!({"schema":"codeclew-cas-object/2.0","objectSchema":"test/1","digest":format!("sha256:{}", "a".repeat(64)),"size":1})).unwrap()
    }
    fn declaration(name: &str, start: usize, end: usize) -> Declaration {
        Declaration {
            compilation: ":/main".into(),
            symbol: format!("callable:p/{name}#jvm:()I"),
            family: Some(format!("p/{name}")),
            kind: "FUNCTION".into(),
            source: SourceAnchor {
                file: "A.kt".into(),
                content: object(),
                start,
                end,
                start_line: 1,
                end_line: 1,
            },
            fact_key: name.into(),
            payload: object(),
            projected_shape: json!({}),
            authority: "COMPILER_PROJECTED_DECLARATION".into(),
            complete_shape: true,
        }
    }
    fn report() -> Comparison {
        let callee = declaration("price", 0, 10);
        let caller = declaration("consumer", 20, 50);
        let relation = Relation {
            compilation: ":/main".into(),
            owner: "p/consumer".into(),
            target: callee.symbol.clone(),
            kind: "CALLS".into(),
            source: SourceAnchor {
                start: 30,
                end: 37,
                ..caller.source.clone()
            },
            fact_key: "call".into(),
            payload: object(),
            exact_call_target: true,
        };
        let analysis = Analysis {
            status: "AVAILABLE".into(),
            authority: "KOTLIN_COMPILER_SUPPORTED_SUBSET".into(),
            ready: None,
            declarations: vec![callee.clone(), caller.clone()],
            relations: vec![relation],
            declaration_coverage_complete: true,
            relation_coverage_complete: true,
            boundaries: vec![],
            failure: None,
        };
        let session = SessionAuthority {
            schema: "test".into(),
            authority_digest: String::new(),
            session_id: String::new(),
            repository_key: String::new(),
            base_revision: String::new(),
            target_ref: String::new(),
            target_oid: String::new(),
            runtime_key: String::new(),
            runtime_mode: RuntimeMode::Development,
            language: SessionLanguage::Kotlin,
            compilations: vec![":/main".into()],
            generation_jobs: None,
            model_cache_policy: ModelCachePolicy::NonCacheable,
            model_cache_authority: None,
            maven_settings_digest: None,
            working_tree: None,
            created_unix_ms: 0,
        };
        let before = Side {
            profile_id: "test".into(),
            session,
            snapshot: object(),
            analysis,
        };
        let mut after = before.clone();
        after.analysis.relations.clear();
        Comparison {
            schema: crate::working_tree_change::SCHEMA.into(),
            comparison_id: String::new(),
            status: "BOUNDED_COMPARISON".into(),
            before,
            after,
            files: vec![],
            declarations: vec![DeclarationChange {
                change_id: "price-change".into(),
                correspondence: "EXACT_SYMBOL_AND_COMPILATION".into(),
                changes: vec!["DECLARATION_SOURCE_TEXT_CHANGED".into()],
                before: Some(callee.clone()),
                after: Some(callee),
                changed_shape_fields: vec![],
                source_text_changed: true,
                behavioral_equivalence: "NOT_PROVEN".into(),
            }],
            unchanged_declaration_count: 1,
            total_changed_file_count: 1,
            total_changed_declaration_count: 1,
            omitted_file_count: 0,
            omitted_declaration_count: 0,
            comparability: String::new(),
            obligations: vec![],
            tests_executed: false,
            consequences: None,
        }
    }
    #[test]
    fn removed_call_keeps_before_evidence_and_invalidates_only_supported_after_claim() {
        let mut report = report();
        let graph = build(&report).unwrap();
        assert_eq!(graph.edges.len(), 1);
        assert_eq!(graph.edges[0].presence, "BEFORE_ONLY");
        assert_eq!(
            graph.edges[0].claim_freshness,
            "BEFORE_RELATION_CLAIM_STALE_FOR_AFTER"
        );
        assert_eq!(graph.candidates.len(), 1);
        assert_eq!(
            graph.candidates[0].authority,
            "STATIC_DERIVED_AFFECTED_CANDIDATE"
        );
        assert!(!graph.tests_executed);
        report.after.analysis.status = "FAILED".into();
        report.after.analysis.relation_coverage_complete = false;
        let graph = build(&report).unwrap();
        assert_eq!(graph.edges[0].claim_freshness, "UNRESOLVED_AFTER_ANALYSIS");
    }
    #[test]
    fn overload_family_is_not_promoted_to_an_exact_target() {
        let mut report = report();
        report.before.analysis.relations[0].target = "p/price".into();
        report.before.analysis.relations[0].exact_call_target = false;
        let graph = build(&report).unwrap();
        assert!(graph.edges.is_empty());
        assert!(graph.candidates.is_empty());
        assert_eq!(graph.unresolved_relation_count, 1);
    }
}
