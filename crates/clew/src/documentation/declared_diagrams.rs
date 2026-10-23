//! Declared architecture is separate from source-resolved transitions.
use super::{
    check::Check,
    external_services, invalid,
    model::{Event, Participant},
    render::escape,
};
use crate::error::ClewError;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn boundary(kind: &str) -> bool {
    matches!(kind, "declared-boundary" | "declared-boundary-return")
}

pub(super) fn participant_href(p: &Participant) -> Option<String> {
    if let Some(id) = &p.external_service {
        Some(format!("../external-services/{id}.html"))
    } else {
        p.service
            .as_ref()
            .map(|id| format!("../services/{id}.html"))
    }
}

fn in_scope(checked: &Check, subject: &str, id: &str, caller: &str) -> bool {
    if let Some(service) = subject.strip_prefix("service:") {
        return service == caller;
    }
    subject.strip_prefix("scenario:").is_some_and(|scenario| {
        external_services::declaration_inputs(checked)
            .and_then(|i| i.scenarios.get(scenario))
            .is_some_and(|s| s.interactions.iter().any(|v| v == id))
    })
}

pub(super) fn validate_arrow(
    event: &Event,
    checked: &Check,
    subject: &str,
) -> Result<(), ClewError> {
    let id = event.interaction.as_deref().ok_or_else(|| invalid("declared boundary needs a delivered interaction reference; read this Work's DECLARED_INTERACTION evidence"))?;
    let dep = checked
        .dependencies
        .get(&format!("interaction:{id}"))
        .filter(|d| d.kind == "DECLARED_INTERACTION")
        .ok_or_else(|| {
            invalid("declared boundary interaction missing; recompose and prepare a fresh Work")
        })?;
    let v = &dep.normalized;
    let ext = v["externalService"].as_str().ok_or_else(|| {
        invalid(
            "declared boundary requires a registered externalService; register it and recompose",
        )
    })?;
    let registry = external_services::for_page(checked, subject);
    if v["external"] != true
        || v["to"]["service"] != ext
        || !registry.as_object().is_some_and(|m| m.contains_key(ext))
        || !v["from"]["service"].as_str().is_some_and(|caller| {
            checked.services.contains_key(caller) && in_scope(checked, subject, id, caller)
        })
    {
        return Err(invalid(
            "declared boundary is outside this subject or disagrees with the registered external destination; select the declared interaction in the process and prepare a fresh Work",
        ));
    }
    if !matches!(v["transport"]["kind"].as_str(), Some("http" | "kafka")) {
        return Err(invalid("declared boundary supports HTTP or Kafka only"));
    }
    if event.kind == "declared-boundary-return" && v["transport"]["kind"] != "http" {
        return Err(invalid(
            "Kafka has no synchronous return; use a separate outgoing reply-event interaction",
        ));
    }
    Ok(())
}

/// Declaration-only evidence is admitted only for boundary events and their
/// exclusive explanation paragraphs. Source-derived messages stay strict.
pub(super) fn supported_refs(
    deps: &[String],
    sources: &[String],
    checked: &Check,
    subject: &str,
) -> Result<(), ClewError> {
    if deps.is_empty() || deps.len() > 128 || sources.len() > 32 {
        return Err(invalid(
            "declared boundary requires bounded retained declaration evidence",
        ));
    }
    let mut supported = BTreeSet::new();
    for id in deps {
        let dep = checked
            .dependencies
            .get(id)
            .ok_or_else(|| invalid("boundary dependency is absent from current context"))?;
        if dep.kind != "DECLARED_INTERACTION" {
            return Err(invalid(
                "boundary meaning must cite only delivered DECLARED_INTERACTION evidence; source behavior belongs in separate source-bound steps",
            ));
        }
        let e = Event {
            id: "evidence".into(),
            kind: "declared-boundary".into(),
            text: String::new(),
            from: None,
            to: None,
            interaction: Some(dep.symbol.clone()),
            dependency_ids: vec![id.clone()],
            source_ids: vec![],
        };
        validate_arrow(&e, checked, subject)?;
        supported.extend(dep.source_ids.iter());
    }
    if sources.iter().any(|id| !supported.contains(id)) {
        return Err(invalid(
            "boundary source is not bound to the declared interaction",
        ));
    }
    Ok(())
}

/// Project immutable catalogue input, never a live repository read or invented call.
pub(super) fn graph(checked: &Check, subject: Option<&str>) -> Value {
    let Some(inputs) = external_services::declaration_inputs(checked) else {
        return json!({"nodes":[],"edges":[]});
    };
    let mut nodes = BTreeMap::new();
    let mut edges = Vec::new();
    for (id, i) in &inputs.interactions {
        if subject.is_some_and(|s| !in_scope(checked, s, id, &i.from.service)) {
            continue;
        }
        let external = i.external_service.as_ref().filter(|ext| {
            i.external && **ext == i.to.service && inputs.external_services.contains_key(*ext)
        });
        if !checked.services.contains_key(&i.from.service)
            || (external.is_none() && !checked.services.contains_key(&i.to.service))
        {
            continue;
        }
        let from = format!("service:{}", i.from.service);
        let to = format!(
            "{}:{}",
            if external.is_some() {
                "external-service"
            } else {
                "service"
            },
            i.to.service
        );
        let a = inputs
            .services
            .get(&i.from.service)
            .map(|s| s.title.as_str())
            .unwrap_or(&i.from.service);
        nodes.insert(from.clone(), json!({"id":from,"service":i.from.service,"label":a,"href":format!("../services/{}.html",i.from.service),"sourceCapture":"CAPTURED"}));
        let b = external
            .and_then(|ext| inputs.external_services.get(ext))
            .map(|s| s.title.as_str())
            .or_else(|| inputs.services.get(&i.to.service).map(|s| s.title.as_str()))
            .unwrap_or(&i.to.service);
        nodes.insert(to.clone(), json!({"id":to,"service":i.to.service,"label":b,"href":format!("../{}/{}.html",if external.is_some(){"external-services"}else{"services"},i.to.service),"sourceCapture":if external.is_some(){"NOT_CAPTURED"}else{"CAPTURED"}}));
        edges.push(json!({"id":id,"from":from,"to":to,"title":i.title,"transport":i.transport,"origin":i.declaration.origin,"declaration":i.declaration,"authority":"DECLARED_RELATIONSHIP","runtime":"UNVERIFIED","sourceChecks":checked.interactions.get(id)}));
    }
    json!({"schema":"codeclew-declared-relationship-graph/1.0","authority":"DECLARED_RELATIONSHIP","runtime":"UNVERIFIED","scope":subject,"nodes":nodes.into_values().collect::<Vec<_>>(),"edges":edges,"executionOrder":"NOT_ESTABLISHED"})
}

fn href(node: &Value, prefix: &str) -> String {
    format!(
        "{}{}",
        prefix,
        node["href"]
            .as_str()
            .unwrap_or("")
            .trim_start_matches("../")
    )
}

/// Inline SVG keeps links active; the adjacent list is the keyboard/text route.
/// Fixed logical width scrolls on narrow screens instead of shrinking labels.
pub(super) fn html(g: &Value, prefix: &str, language: &str) -> String {
    let nodes = g["nodes"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    let edges = g["edges"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    if edges.is_empty() {
        return String::new();
    }
    let destinations: BTreeSet<_> = edges.iter().filter_map(|e| e["to"].as_str()).collect();
    let mut positions = BTreeMap::new();
    let mut columns = [0usize; 2];
    for node in nodes {
        let col = usize::from(destinations.contains(node["id"].as_str().unwrap_or("")));
        positions.insert(
            node["id"].as_str().unwrap_or(""),
            (if col == 0 { 160 } else { 840 }, 75 + columns[col] * 145),
        );
        columns[col] += 1;
    }
    let first_edge_y = positions.values().map(|(_, y)| *y).max().unwrap_or(0) / 4 + 60;
    let height = (columns.into_iter().max().unwrap_or(1) * 145 + 60)
        .max(first_edge_y + edges.len() * 62 + 110);
    let mut svg = format!(
        "<div style=\"overflow-x:auto\"><svg class=\"declared-graph\" style=\"min-width:1000px;width:100%\" viewBox=\"0 0 1000 {height}\" role=\"group\" aria-label=\"Declared service relationships\"><title>Declared relationships; execution order is not established</title><defs><marker id=\"declared-arrow\" viewBox=\"0 0 8 8\" refX=\"7\" refY=\"4\" markerWidth=\"7\" markerHeight=\"7\" orient=\"auto\"><path d=\"M0 0 L8 4 L0 8Z\" fill=\"#526879\"/></marker></defs>"
    );
    for (index, e) in edges.iter().enumerate() {
        let (Some(&(ax, ay)), Some(&(bx, by))) = (
            positions.get(e["from"].as_str().unwrap_or("")),
            positions.get(e["to"].as_str().unwrap_or("")),
        ) else {
            continue;
        };
        let y = first_edge_y + index * 62;
        // A cubic midpoint is (ay + 6*control_y + by)/8. Put its label there.
        let control_y = (8.0 * y as f64 - ay as f64 - by as f64) / 6.0;
        let control_x = (8.0 * 500.0 - (ax + 130) as f64 - (bx - 130) as f64) / 6.0;
        svg += &format!(
            "<g data-interaction=\"{}\"><title>{}</title><path d=\"M{} {ay} C{control_x:.2} {control_y:.2} {control_x:.2} {control_y:.2} {} {by}\" fill=\"none\" stroke=\"#526879\" stroke-dasharray=\"6 4\" marker-end=\"url(#declared-arrow)\"/><rect x=\"340\" y=\"{}\" width=\"320\" height=\"40\" rx=\"4\" fill=\"#fffdf8\"/><text x=\"500\" y=\"{y}\" text-anchor=\"middle\" font-size=\"13\">{} · {}</text><text x=\"500\" y=\"{}\" text-anchor=\"middle\" font-size=\"11\">{} · runtime unverified</text></g>",
            escape(e["id"].as_str().unwrap_or("")),
            escape(e["title"].as_str().unwrap_or("")),
            ax + 130,
            bx - 130,
            y.saturating_sub(18),
            escape(e["id"].as_str().unwrap_or("")),
            escape(e["transport"]["kind"].as_str().unwrap_or("")),
            y + 16,
            escape(e["origin"].as_str().unwrap_or(""))
        );
    }
    let mut links = String::new();
    for node in nodes {
        let (x, y) = positions[node["id"].as_str().unwrap_or("")];
        let url = escape(&href(node, prefix));
        let label = node["label"].as_str().unwrap_or("");
        let chunks = label
            .chars()
            .collect::<Vec<_>>()
            .chunks(28)
            .map(|c| c.iter().collect::<String>())
            .take(3)
            .collect::<Vec<_>>();
        svg += &format!(
            "<a href=\"{url}\" aria-label=\"{}\"><title>{}</title><rect x=\"{}\" y=\"{}\" width=\"260\" height=\"112\" rx=\"8\" fill=\"#eef4ef\" stroke=\"#607c69\"/>",
            escape(label),
            escape(label),
            x - 130,
            y.saturating_sub(40)
        );
        for (n, line) in chunks.iter().enumerate() {
            svg += &format!(
                "<text x=\"{x}\" y=\"{}\" text-anchor=\"middle\" font-size=\"15\">{}</text>",
                y - 15 + n * 19,
                escape(line)
            );
        }
        svg += &format!(
            "<text x=\"{x}\" y=\"{}\" text-anchor=\"middle\" font-size=\"11\">{}</text></a>",
            y + 57,
            escape(node["sourceCapture"].as_str().unwrap_or(""))
        );
        links += &format!(
            "<li><a href=\"{url}\">{}</a> · <code>{}</code> · {}</li>",
            escape(label),
            escape(node["id"].as_str().unwrap_or("")),
            escape(node["sourceCapture"].as_str().unwrap_or(""))
        );
    }
    svg += "</svg></div>";
    let rows=edges.iter().map(|e|format!("<li><code>{}</code>: {} → {} · {} · origin {} · declared only; runtime unverified<details><summary>Original declaration and source checks</summary><p>{}</p><pre>{}</pre></details></li>",escape(e["id"].as_str().unwrap_or("")),escape(e["from"].as_str().unwrap_or("")),escape(e["to"].as_str().unwrap_or("")),escape(e["transport"]["kind"].as_str().unwrap_or("")),escape(e["origin"].as_str().unwrap_or("")),escape(e["declaration"]["rationale"].as_str().unwrap_or("")),escape(&serde_json::to_string_pretty(e).unwrap_or_default()))).collect::<String>();
    let heading = super::language::label(language, "Declared service relationships");
    let note = super::language::label(
        language,
        "Declared architecture, not an observed execution trace. Source checks remain separate; external source is not captured. Agent proposals are unverified. Position does not establish execution order.",
    );
    format!(
        "<section class=\"declared-relationships\"><h2>{heading}</h2><p>{note}</p>{svg}<details><summary>Participants and relationships as text</summary><ul>{links}</ul><ul>{rows}</ul></details></section>"
    )
}

pub(super) fn markdown(g: &Value) -> String {
    if g["edges"].as_array().is_none_or(Vec::is_empty) {
        return String::new();
    }
    let mut out="\n## Declared service relationships\n\nDeclared architecture; runtime unverified. Position does not establish execution order. External source is not captured.\n\n".to_owned();
    for node in g["nodes"].as_array().into_iter().flatten() {
        out += &format!(
            "- [{}](../{}) · `{}` · {}\n",
            escape(node["label"].as_str().unwrap_or("")).replace(['[', ']'], ""),
            href(node, ""),
            node["id"],
            node["sourceCapture"]
        );
    }
    for edge in g["edges"].as_array().into_iter().flatten() {
        out += &format!(
            "- `{}`: {} → {} · {} · origin {} · declared only; runtime unverified\n",
            edge["id"], edge["from"], edge["to"], edge["transport"]["kind"], edge["origin"]
        );
    }
    out
}

pub(super) fn mermaid(g: &Value) -> String {
    let mut out="flowchart LR\n    %% Declared architecture; external source not captured; runtime unverified; no execution order.\n".to_owned();
    let mut ids = BTreeMap::new();
    for (index, node) in g["nodes"].as_array().into_iter().flatten().enumerate() {
        let alias = format!("n{index}");
        ids.insert(node["id"].as_str().unwrap_or(""), alias.clone());
        // JSON string quoting is not enough for Mermaid: encode directive characters.
        let label = escape(node["label"].as_str().unwrap_or(""))
            .replace(';', "&#59;")
            .replace(['\n', '\r'], " ");
        out += &format!(
            "    {alias}[\"{label}\"]\n    click {alias} \"{}\" \"Documentation\"\n",
            node["href"].as_str().unwrap_or("")
        );
    }
    for edge in g["edges"].as_array().into_iter().flatten() {
        if let (Some(a), Some(b)) = (
            ids.get(edge["from"].as_str().unwrap_or("")),
            ids.get(edge["to"].as_str().unwrap_or("")),
        ) {
            out += &format!(
                "    {a} -.\"{} · {} · declared; runtime unverified\".-> {b}\n",
                escape(edge["id"].as_str().unwrap_or("")).replace(';', "&#59;"),
                edge["transport"]["kind"].as_str().unwrap_or("")
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Check, Event) {
        let service = json!({"schema":"codeclew-documentation-service/1.0","id":"owned","title":"Same <title>","repositoryId":"owned","repository":"https://example.invalid/owned","language":"java","profile":"source-syntax","targetRef":"main","source":{"roots":["src"],"dialect":"17"}});
        let ext = |id: &str| json!({"schema":"codeclew-documentation-external-service/1.0","id":id,"title":"Same <title>","description":"Uncaptured","declaration":{"origin":"agent-proposal","rationale":"Unverified"},"limitations":[]});
        let interaction = |id: &str, target: &str, kind: &str| json!({"schema":"codeclew-documentation-interaction/1.0","id":id,"title":"Declared","from":{"service":"owned"},"to":{"service":target},"external":true,"externalService":target,"transport":{"kind":kind},"declaration":{"origin":"agent-proposal","rationale":"Not observed"}});
        let a = interaction("http", "provider", "http");
        let b = interaction("event", "provider", "kafka");
        let c = interaction("unselected", "other", "http");
        let deps=[("http",a.clone()),("event",b.clone()),("unselected",c.clone())].into_iter().map(|(id,row)|(format!("interaction:{id}"),json!({"id":format!("interaction:{id}"),"kind":"DECLARED_INTERACTION","service":"owned","symbol":id,"normalized":row,"digest":"d","sourceIds":[]}))).collect::<serde_json::Map<_,_>>();
        let checked=serde_json::from_value(json!({"schema":"test","inputDigest":"i","contextDigest":"c","services":{"owned":{"schema":"evidence","service":"owned","revision":"rev","serviceDigest":"d","extractor":"source","runtimeMode":"SOURCE_SYNTAX","coverage":"SYNTAX","boundaries":[],"entrypoints":[],"observations":{},"sources":{},"contracts":{}}},"unresolved":{},"interactions":{},"scenarios":{},"dependencies":deps,"sourceInputs":{"schema":"codeclew-documentation-source-inputs/1.0","inputDigest":"i","selectedServices":["owned"],"inputs":{"manifest":{"schema":"docs","title":"Fixture"},"services":{"owned":service},"externalServices":{"provider":ext("provider"),"other":ext("other")},"interactions":{"http":a,"event":b,"unselected":c},"scenarios":{"process":{"schema":"codeclew-documentation-process/1.0","id":"process","title":"Selected","summary":"Declared only","root":{"service":"owned"},"interactions":["http","event"]}},"entities":{},"notes":{},"evidenceExpectations":{},"updatePolicies":{},"updateState":{"schema":"state","targets":{}}}}})).unwrap();
        let event=serde_json::from_value(json!({"id":"step","kind":"declared-boundary","text":"Declared request","interaction":"http","from":"service-owned","to":"provider","dependencyIds":["interaction:http"],"sourceIds":[]})).unwrap();
        (checked, event)
    }
    #[test]
    fn boundary_is_scoped_registered_and_never_requires_resolved_source_call() {
        let (mut checked, mut e) = fixture();
        let before = serde_json::to_value(&checked).unwrap();
        validate_arrow(&e, &checked, "scenario:process").unwrap();
        validate_arrow(&e, &checked, "service:owned").unwrap();
        assert!(validate_arrow(&e, &checked, "service:missing").is_err());
        e.interaction = Some("unselected".into());
        assert!(validate_arrow(&e, &checked, "scenario:process").is_err());
        e.interaction = Some("http".into());
        assert_eq!(serde_json::to_value(&checked).unwrap(), before);
        checked
            .dependencies
            .get_mut("interaction:http")
            .unwrap()
            .normalized["to"]["service"] = json!("other");
        assert!(validate_arrow(&e, &checked, "scenario:process").is_err());
    }
    #[test]
    fn http_response_is_explicit_and_kafka_never_gets_a_synchronous_return() {
        let (checked, mut e) = fixture();
        e.kind = "declared-boundary-return".into();
        validate_arrow(&e, &checked, "scenario:process").unwrap();
        e.interaction = Some("event".into());
        assert!(
            validate_arrow(&e, &checked, "scenario:process")
                .unwrap_err()
                .message
                .contains("Kafka")
        );
        e.kind = "declared-boundary".into();
        validate_arrow(&e, &checked, "scenario:process").unwrap();
    }
    #[test]
    fn scoped_graph_keeps_parallel_ids_and_same_titles_separate_and_links_safe() {
        let (checked, _) = fixture();
        let g = graph(&checked, Some("scenario:process"));
        assert_eq!(g["nodes"].as_array().unwrap().len(), 2);
        assert_eq!(g["edges"].as_array().unwrap().len(), 2);
        assert_eq!(g["nodes"][0]["id"], "external-service:provider");
        assert_eq!(g["nodes"][1]["id"], "service:owned");
        assert_ne!(g["edges"][0]["id"], g["edges"][1]["id"]);
        let page = html(&g, "../", "en");
        assert!(page.contains("href=\"../external-services/provider.html\""));
        assert!(!page.contains("Same <title>"));
        assert!(page.contains("agent-proposal"));
        assert!(page.contains("Original declaration and source checks"));
        assert!(page.contains("Not observed"));
        assert!(mermaid(&g).contains("click n0"));
        assert!(markdown(&g).contains("../external-services/provider.html"));
        assert_eq!(graph(&checked, Some("scenario:empty"))["nodes"], json!([]));
        assert_eq!(
            graph(&checked, Some("service:owned"))["edges"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
    }
    #[test]
    fn full_narrative_accepts_only_declaration_bound_steps_without_fake_sources() {
        let (mut checked, e) = fixture();
        checked.scenarios.insert("process".into(),serde_json::from_value(json!({"id":"process","steps":[{"id":"root","service":"owned","symbol":"submit","kind":"SOURCE_DECLARATION","dependencyIds":["method"],"sourceIds":["source"],"detail":{},"depth":0}],"boundaries":[],"truncated":false,"dependencyIds":[]})).unwrap());
        checked.dependencies.insert("method".into(),serde_json::from_value(json!({"id":"method","kind":"SYMBOL","service":"owned","symbol":"submit","digest":"d","sourceIds":["source"],"normalized":{}})).unwrap());
        let n:super::super::model::Narrative=serde_json::from_value(json!({"schema":"codeclew-documentation-narrative/1.4","subject":"scenario:process","contextDigest":"c","operations":[{"id":"process","title":"Declared boundary","summary":{"id":"summary","text":"Source root with a declared boundary.","dependencyIds":["method"],"sourceIds":["source"]},"participants":[{"id":"service-owned","label":"Owned","service":"owned"},{"id":"provider","label":"Provider","service":null,"externalService":"provider"}],"events":[e],"explanation":[{"id":"explanation","text":"Declared only; receiver execution is unknown.","eventIds":["step"],"dependencyIds":["interaction:http"],"sourceIds":[],"detail":false}]}],"gaps":{}})).unwrap();
        super::super::render::validate(&n, &checked).unwrap();
        let mut old = n.clone();
        old.schema = "codeclew-documentation-narrative/1.3".into();
        assert!(super::super::render::validate(&old, &checked).is_err());
        let mut wrong = n.clone();
        wrong.operations[0].events[0].to = Some("service-owned".into());
        assert!(super::super::render::validate(&wrong, &checked).is_err());
        let mut message = n.clone();
        message.operations[0].events[0].kind = "message".into();
        assert!(super::super::render::validate(&message, &checked).is_err());
        let mut both = n.clone();
        both.operations[0].participants[1].service = Some("owned".into());
        assert!(super::super::render::validate(&both, &checked).is_err());
    }

    #[test]
    fn legacy_participant_bytes_stay_unchanged_and_declared_schema_is_explicit() {
        let p: Participant =
            serde_json::from_value(json!({"id":"actor","label":"Actor","service":null})).unwrap();
        assert_eq!(
            serde_json::to_value(p).unwrap(),
            json!({"id":"actor","label":"Actor","service":null})
        );
    }
}
