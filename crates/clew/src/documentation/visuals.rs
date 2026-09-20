//! Evidence-bound visual artifacts. Layout is renderer-owned; author input is data only.
use super::{invalid, model::Fragment, proposals::Claim, store};
use crate::error::ClewError;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const SCHEMA: &str = "codeclew-documentation-visual/1.0";
pub const GENERATOR: &str = "typed-visual/1.0";
const MAX_ARTIFACTS: usize = 32;
const MAX_NODES: usize = 64;
const MAX_EDGES: usize = 128;
const MAX_RULES: usize = 128;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Parent {
    pub artifact: String,
    pub node: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProposedVisual {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub purpose: Claim,
    pub scope: Claim,
    pub limitations: Vec<String>,
    #[serde(default)]
    pub nodes: Vec<ProposedNode>,
    #[serde(default)]
    pub edges: Vec<ProposedEdge>,
    #[serde(default)]
    pub parent: Option<Parent>,
    #[serde(default)]
    pub hit_policy: Option<String>,
    #[serde(default)]
    pub policy_explanation: Option<Claim>,
    #[serde(default)]
    pub rules: Vec<ProposedRule>,
    #[serde(default)]
    pub after_selection: Option<Claim>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProposedNode {
    pub id: String,
    pub meaning: Claim,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProposedEdge {
    pub id: String,
    pub from: String,
    pub to: String,
    pub meaning: Claim,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProposedRule {
    pub condition: Claim,
    pub outcome: Claim,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Visual {
    pub schema: String,
    pub generator: String,
    pub id: String,
    pub kind: String,
    pub title: String,
    pub purpose: Fragment,
    pub scope: Fragment,
    pub limitations: Vec<String>,
    #[serde(default)]
    pub nodes: Vec<Node>,
    #[serde(default)]
    pub edges: Vec<Edge>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<Parent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hit_policy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_explanation: Option<Fragment>,
    #[serde(default)]
    pub rules: Vec<Rule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_selection: Option<Fragment>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Node {
    pub id: String,
    pub meaning: Fragment,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Edge {
    pub id: String,
    pub from: String,
    pub to: String,
    pub meaning: Fragment,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Rule {
    pub condition: Fragment,
    pub outcome: Fragment,
}

/// Every authored semantic field passes through the caller's evidence resolver.
/// Slash-separated slots cannot collide because artifact/node IDs forbid slashes.
pub fn materialize(
    input: &[ProposedVisual],
    mut claim: impl FnMut(&str, &Claim) -> Result<Fragment, ClewError>,
) -> Result<Vec<Visual>, ClewError> {
    if input.len() > MAX_ARTIFACTS {
        return Err(invalid(
            "at most 32 visual artifacts may be attached to one operation",
        ));
    }
    let mut output = Vec::with_capacity(input.len());
    for p in input {
        if !store::valid_id(&p.id)
            || !bounded_text(&p.title, 256)
            || p.limitations.is_empty()
            || p.limitations.len() > 32
            || p.limitations.iter().any(|text| !bounded_text(text, 2048))
            || p.nodes.len() > MAX_NODES
            || p.edges.len() > MAX_EDGES
            || p.rules.len() > MAX_RULES
        {
            return Err(invalid(
                "visual identity, title, explicit limitations or item bounds are invalid",
            ));
        }
        let mut limitations = p.limitations.clone();
        let mut resolve = |slot: &str, value: &Claim| {
            if !bounded_text(&value.text, 4096)
                || value.evidence.is_empty()
                || value.evidence.len() > 32
            {
                return Err(invalid(
                    "every visual claim requires bounded text and explicit evidence",
                ));
            }
            if let Some(uncertainty) = &value.uncertainty {
                if !bounded_text(uncertainty, 2048) {
                    return Err(invalid(
                        "visual claim uncertainty requires bounded nonempty text",
                    ));
                }
                if !limitations.contains(uncertainty) {
                    limitations.push(uncertainty.clone());
                }
            }
            claim(&format!("visual/{}/{slot}", p.id), value)
        };
        let purpose = resolve("purpose", &p.purpose)?;
        let scope = resolve("scope", &p.scope)?;
        let mut nodes = Vec::with_capacity(p.nodes.len());
        for n in &p.nodes {
            nodes.push(Node {
                id: n.id.clone(),
                meaning: resolve(&format!("node/{}", n.id), &n.meaning)?,
            });
        }
        let mut edges = Vec::with_capacity(p.edges.len());
        for e in &p.edges {
            edges.push(Edge {
                id: e.id.clone(),
                from: e.from.clone(),
                to: e.to.clone(),
                meaning: resolve(&format!("edge/{}", e.id), &e.meaning)?,
            });
        }
        let mut rules = Vec::with_capacity(p.rules.len());
        for (i, r) in p.rules.iter().enumerate() {
            rules.push(Rule {
                condition: resolve(&format!("rule/{i}/condition"), &r.condition)?,
                outcome: resolve(&format!("rule/{i}/outcome"), &r.outcome)?,
            });
        }
        let policy_explanation = p
            .policy_explanation
            .as_ref()
            .map(|value| resolve("policy-explanation", value))
            .transpose()?;
        let after_selection = p
            .after_selection
            .as_ref()
            .map(|value| resolve("after-selection", value))
            .transpose()?;
        output.push(Visual {
            schema: SCHEMA.into(),
            generator: GENERATOR.into(),
            id: p.id.clone(),
            kind: p.kind.clone(),
            title: p.title.clone(),
            purpose,
            scope,
            limitations,
            nodes,
            edges,
            parent: p.parent.clone(),
            hit_policy: p.hit_policy.clone(),
            policy_explanation,
            rules,
            after_selection,
        });
    }
    validate_structure(&output)?;
    Ok(output)
}

pub fn fragments(visual: &Visual) -> Vec<&Fragment> {
    let mut all = vec![&visual.purpose, &visual.scope];
    all.extend(visual.nodes.iter().map(|n| &n.meaning));
    all.extend(visual.edges.iter().map(|e| &e.meaning));
    for rule in &visual.rules {
        all.extend([&rule.condition, &rule.outcome]);
    }
    all.extend(visual.policy_explanation.as_ref());
    all.extend(visual.after_selection.as_ref());
    all
}

fn bounded_text(text: &str, max: usize) -> bool {
    !text.trim().is_empty()
        && text.len() <= max
        && !text
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
}

/// Validate persisted structure as well as author input. Evidence authority and
/// fragment bindings are checked by the enclosing operation's lifecycle validator.
pub fn validate_structure(visuals: &[Visual]) -> Result<(), ClewError> {
    if visuals.len() > MAX_ARTIFACTS {
        return Err(invalid("too many visual artifacts in one operation"));
    }
    let mut artifacts = BTreeMap::new();
    let mut claims = BTreeSet::new();
    for v in visuals {
        if v.schema != SCHEMA
            || v.generator != GENERATOR
            || !store::valid_id(&v.id)
            || artifacts.insert(v.id.as_str(), v).is_some()
            || !bounded_text(&v.title, 256)
            || v.limitations.is_empty()
            || v.limitations.len() > 32
            || v.limitations.iter().any(|text| !bounded_text(text, 2048))
            || v.nodes.len() > MAX_NODES
            || v.edges.len() > MAX_EDGES
            || v.rules.len() > MAX_RULES
        {
            return Err(invalid(
                "visual schema, generator, identity, explicit limitations or bounds are invalid",
            ));
        }
        for f in fragments(v) {
            if !store::valid_id(&f.id)
                || !claims.insert(f.id.as_str())
                || !bounded_text(&f.text, 4096)
            {
                return Err(invalid(
                    "visual fragments require unique bounded identities and nonempty text",
                ));
            }
        }
        match v.kind.as_str() {
            "execution-flow" | "dependency-map" => {
                if v.nodes.is_empty()
                    || v.parent.is_some()
                    || v.hit_policy.is_some()
                    || v.policy_explanation.is_some()
                    || !v.rules.is_empty()
                    || v.after_selection.is_some()
                {
                    return Err(invalid(
                        "graph visuals require nodes and cannot contain decision-table fields",
                    ));
                }
            }
            "decision-table" => {
                if !v.nodes.is_empty()
                    || !v.edges.is_empty()
                    || v.rules.is_empty()
                    || v.policy_explanation.is_none()
                    || !matches!(
                        v.hit_policy.as_deref(),
                        Some("FIRST" | "UNIQUE" | "UNKNOWN")
                    )
                {
                    return Err(invalid(
                        "decision tables require FIRST, UNIQUE or UNKNOWN with an evidence-bound policy explanation, rules, and no graph fields",
                    ));
                }
            }
            _ => return Err(invalid("unsupported typed visual kind")),
        }
        let mut ids = BTreeSet::new();
        let mut node_ids = BTreeSet::new();
        for n in &v.nodes {
            if !store::valid_id(&n.id)
                || !ids.insert(n.id.as_str())
                || !bounded_text(&n.meaning.text, 1024)
            {
                return Err(invalid(
                    "visual node identity or label is invalid or duplicated",
                ));
            }
            node_ids.insert(n.id.as_str());
        }
        for e in &v.edges {
            if !store::valid_id(&e.id)
                || !ids.insert(e.id.as_str())
                || !node_ids.contains(e.from.as_str())
                || !node_ids.contains(e.to.as_str())
                || !bounded_text(&e.meaning.text, 1024)
            {
                return Err(invalid(
                    "visual edge has invalid or duplicate identity, dangling endpoint or oversized label",
                ));
            }
        }
    }
    for v in visuals {
        if let Some(parent) = &v.parent {
            let graph = artifacts.get(parent.artifact.as_str()).ok_or_else(|| {
                invalid("decision parent artifact is missing from the same operation")
            })?;
            if !store::valid_id(&parent.artifact)
                || !store::valid_id(&parent.node)
                || graph.kind != "execution-flow"
                || !graph.nodes.iter().any(|node| node.id == parent.node)
            {
                return Err(invalid(
                    "decision parent must name an existing execution-flow node in the same operation",
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn claim(text: &str) -> Value {
        json!({"text":text,"evidence":["e1"]})
    }
    fn graph() -> Value {
        json!({"id":"dispatch","kind":"execution-flow","title":"Dispatch flow",
            "purpose":claim("Show dispatch order"),"scope":claim("Within dispatch(request)"),
            "limitations":["Static source interpretation; remote completion is not established."],
            "nodes":[{"id":"select","meaning":claim("Select action")},
                {"id":"execute","meaning":claim("Execute selected action")}],
            "edges":[{"id":"selected","from":"select","to":"execute","meaning":claim("After selection returns")}]
        })
    }
    fn table() -> Value {
        json!({"id":"policy","kind":"decision-table","title":"Dispatch decision",
            "purpose":claim("Explain which action is selected"),"scope":claim("One invocation of select(request)"),
            "limitations":["Only inspected branches are shown."],
            "parent":{"artifact":"dispatch","node":"select"},"hitPolicy":"FIRST",
            "policyExplanation":claim("Ordered conditions select the first matching branch"),
            "rules":[{"condition":claim("request is valid"),"outcome":claim("Select send action")}],
            "afterSelection":claim("The caller executes the action; send can fail")
        })
    }
    fn build(values: Vec<Value>) -> Result<Vec<Visual>, ClewError> {
        let input: Vec<ProposedVisual> = serde_json::from_value(json!(values)).unwrap();
        materialize(&input, |id, value| {
            Ok(Fragment {
                id: format!("claim-{}", &super::super::digest(&id)?[7..27]),
                text: value.text.clone(),
                dependency_ids: vec!["source:one".into()],
                source_ids: vec!["source-one".into()],
            })
        })
    }
    #[test]
    fn linked_flow_and_decision_materialize_all_claims() {
        let result = build(vec![graph(), table()]).unwrap();
        assert_eq!(fragments(&result[0]).len(), 5);
        assert_eq!(fragments(&result[1]).len(), 6);
        assert_eq!(result[1].parent.as_ref().unwrap().node, "select");
        assert_eq!(result[1].schema, SCHEMA);
        assert_eq!(result[1].generator, GENERATOR);
        let roundtrip: Vec<Visual> = serde_json::from_value(json!(result)).unwrap();
        assert_eq!(result, roundtrip);
    }
    #[test]
    fn local_decision_requires_scope_but_not_a_fabricated_parent() {
        let mut local = table();
        local.as_object_mut().unwrap().remove("parent");
        local["hitPolicy"] = json!("UNIQUE");
        assert!(build(vec![local.clone()]).is_ok());
        local["scope"]["text"] = json!("");
        assert!(build(vec![local]).is_err());
    }
    #[test]
    fn dangling_edges_and_parents_are_rejected() {
        let mut bad = graph();
        bad["edges"][0]["to"] = json!("missing");
        assert!(build(vec![bad]).is_err());
        assert!(build(vec![table()]).is_err());
        let mut bad = table();
        bad["parent"]["node"] = json!("missing");
        assert!(build(vec![graph(), bad]).is_err());
        let mut bad = table();
        bad["parent"]["artifact"] = json!("policy");
        assert!(build(vec![graph(), bad]).is_err());
    }
    #[test]
    fn unsafe_duplicate_and_cross_kind_fields_are_rejected() {
        for id in ["../outside", "\" onclick=", "space id", "évil", "1first"] {
            let mut bad = graph();
            bad["id"] = json!(id);
            assert!(build(vec![bad]).is_err());
        }
        assert!(build(vec![graph(), graph()]).is_err());
        let mut bad = graph();
        bad["edges"][0]["id"] = json!("select");
        assert!(build(vec![bad]).is_err());
        let mut bad = graph();
        bad["nodes"][1]["id"] = json!("select");
        assert!(build(vec![bad]).is_err());
        for policy in ["ANY", "COLLECT", "first", ""] {
            let mut bad = table();
            bad["hitPolicy"] = json!(policy);
            assert!(build(vec![graph(), bad]).is_err());
        }
        let mut bad = graph();
        bad["hitPolicy"] = json!("FIRST");
        assert!(build(vec![bad]).is_err());
        let mut bad = table();
        bad["nodes"] = graph()["nodes"].clone();
        assert!(build(vec![graph(), bad]).is_err());
    }
    #[test]
    fn missing_evidence_limits_or_unsupported_fields_are_rejected() {
        for path in ["purpose", "scope"] {
            let mut bad = graph();
            bad[path]["evidence"] = json!([]);
            assert!(build(vec![bad]).is_err());
        }
        let mut bad = graph();
        bad["limitations"] = json!([]);
        bad["scope"]["uncertainty"] = json!("Only one branch was inspected");
        assert!(build(vec![bad]).is_err());
        let mut bad = table();
        bad["afterSelection"]["evidence"] = json!([]);
        assert!(build(vec![graph(), bad]).is_err());
        let mut bad = graph();
        bad["mermaid"] = json!("flowchart TD");
        assert!(serde_json::from_value::<ProposedVisual>(bad).is_err());
        let mut bad = graph();
        bad.as_object_mut().unwrap().remove("limitations");
        assert!(serde_json::from_value::<ProposedVisual>(bad).is_err());
    }
    #[test]
    fn unknown_policy_preserves_abstention_and_dependency_maps_are_not_invocations() {
        let mut unknown = table();
        unknown["hitPolicy"] = json!("UNKNOWN");
        unknown["policyExplanation"] =
            claim("The available evidence does not establish selection order");
        assert!(build(vec![graph(), unknown]).is_ok());
        let mut map = graph();
        map["kind"] = json!("dependency-map");
        assert!(build(vec![map.clone()]).is_ok());
        assert!(build(vec![map, table()]).is_err());
        let mut unexplained = table();
        unexplained
            .as_object_mut()
            .unwrap()
            .remove("policyExplanation");
        assert!(build(vec![graph(), unexplained]).is_err());
    }
    #[test]
    fn every_semantic_field_requires_evidence_and_resolver_errors_propagate() {
        for pointer in ["/purpose", "/scope", "/nodes/0/meaning", "/edges/0/meaning"] {
            let mut bad = graph();
            bad.pointer_mut(pointer).unwrap()["evidence"] = json!([]);
            assert!(build(vec![bad]).is_err(), "{pointer}");
        }
        for pointer in [
            "/purpose",
            "/scope",
            "/rules/0/condition",
            "/rules/0/outcome",
            "/policyExplanation",
            "/afterSelection",
        ] {
            let mut bad = table();
            bad.pointer_mut(pointer).unwrap()["evidence"] = json!([]);
            assert!(build(vec![graph(), bad]).is_err(), "{pointer}");
        }
        let input: Vec<ProposedVisual> = serde_json::from_value(json!([graph()])).unwrap();
        let failure = materialize(&input, |_, _| Err(invalid("missing saved source"))).unwrap_err();
        assert!(failure.message.contains("missing saved source"));
    }
    #[test]
    fn claim_uncertainties_are_retained_once_as_reader_visible_limitations() {
        let mut input = graph();
        input["scope"]["uncertainty"] = json!("Dynamic selection has not been observed.");
        input["nodes"][0]["meaning"]["uncertainty"] = input["scope"]["uncertainty"].clone();
        let result = build(vec![input]).unwrap();
        assert_eq!(result[0].limitations.len(), 2);
        assert_eq!(
            result[0].limitations[1],
            "Dynamic selection has not been observed."
        );
    }
    #[test]
    fn persisted_artifact_validation_checks_schema_bounds_and_claim_identity() {
        let good = build(vec![graph(), table()]).unwrap();
        let mut bad = good.clone();
        bad[0].schema = "other".into();
        assert!(validate_structure(&bad).is_err());
        let mut bad = good.clone();
        bad[0].generator = "untrusted".into();
        assert!(validate_structure(&bad).is_err());
        let mut bad = good.clone();
        bad[0].nodes[0].meaning.id = bad[1].scope.id.clone();
        assert!(validate_structure(&bad).is_err());
        let mut bad = good.clone();
        bad[0].nodes[0].meaning.text = "x".repeat(1025);
        assert!(validate_structure(&bad).is_err());
        let mut bad = good;
        bad[0].parent = Some(Parent {
            artifact: "dispatch".into(),
            node: "select".into(),
        });
        assert!(validate_structure(&bad).is_err());
    }
}
