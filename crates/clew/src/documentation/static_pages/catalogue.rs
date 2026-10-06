//! Discovery of this immutable native bundle, never repository-wide impact.
use super::super::model::*;
use super::{anchor, callable_label, escape, process_label};
use crate::{documentation::digest, error::ClewError};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(super) struct CatalogueLink {
    pub href: String,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(super) struct Row {
    pub id: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_title: Option<String>,
    pub kind: String,
    pub href: String,
    pub context: String,
    pub summary: String,
    pub search_text: Vec<String>,
    pub related_links: Vec<CatalogueLink>,
}

pub(super) fn rows(p: &BundleProjection) -> Result<Vec<Row>, ClewError> {
    let mut rows = BTreeMap::new();
    let pages: BTreeMap<_, _> = p
        .pages
        .iter()
        .map(|page| (page.id.as_str(), page))
        .collect();
    for page in pages.values() {
        let scope = page
            .observations
            .get(&page.endpoint.declaration_id)
            .and_then(|o| o.normalized["scope"].as_str())
            .unwrap_or("unavailable scope");
        let context = format!("{} · {scope}", page.selection.service);
        if page
            .projection_kind
            .is_some_and(ProjectionKind::is_declaration_view)
        {
            let mut search_text = vec![
                page.selection.service.clone(),
                scope.into(),
                page.endpoint.symbol.clone(),
                page.worker.symbol.clone(),
            ];
            if let Some(wiring) = &page.wiring {
                search_text.push(wiring.symbol.clone());
            }
            rows.insert(
                format!("declarations:{}", page.id),
                Row {
                    id: format!("declarations-{}", page.id),
                    title: page.title.clone(),
                    display_title: None,
                    kind: "Selected declarations".into(),
                    href: format!("{}-overview.html", page.id),
                    context: context.clone(),
                    summary: if page.projection_kind == Some(ProjectionKind::CompilerControlFlow) {
                        "Retained compiler-bound declarations and function-local control-flow graphs; inter-function relationships, execution order and runtime behavior are not inferred.".into()
                    } else if page.endpoint.authority == "SYNTAX_DECLARATION" || page.worker.authority == "SYNTAX_DECLARATION" {
                        "Retained syntax declarations, signatures and cited source structure; compiler resolution, behavior and relationships are not inferred.".into()
                    } else {
                        "Retained compiler-bound declarations and source; behavior and relationships are not inferred.".into()
                    },
                    search_text: search_text.clone(),
                    related_links: vec![],
                },
            );
            if let Some(question) = &page.selection.question {
                rows.insert(
                    format!("question:{}", page.id),
                    Row {
                        id: format!("question-{}", page.id),
                        title: question.clone(),
                        display_title: None,
                        kind: "Selected question".into(),
                        href: format!("{}-diagnostic.html", page.id),
                        context,
                        summary: "Question text supplied with the selected declarations; no source-condition analysis was projected.".into(),
                        search_text,
                        related_links: vec![],
                    },
                );
            }
            continue;
        }
        rows.insert(
            format!("process:{}", page.id),
            Row {
                id: page.id.clone(),
                title: process_label(page),
                display_title: None,
                kind: "Process".into(),
                href: format!("{}-overview.html", page.id),
                context: context.clone(),
                summary:
                    "Selected source process; runtime activation and business meaning unverified."
                        .into(),
                search_text: vec![
                    page.selection.service.clone(),
                    scope.into(),
                    page.endpoint.symbol.clone(),
                    page.worker.symbol.clone(),
                ],
                related_links: vec![],
            },
        );
        let endpoint_key = format!(
            "endpoint-{}",
            &digest(&(&page.selection.service, scope, &page.endpoint.symbol))?[7..]
        );
        let endpoint = rows.entry(endpoint_key.clone()).or_insert_with(|| Row {
            id: endpoint_key, title: page.endpoint.symbol.clone(),
            display_title: Some(callable_label(&page.endpoint.symbol)), kind: "Endpoint".into(),
            href: format!("{}-endpoint.html", page.id), context,
            summary: "Explicitly selected compiler declaration; no inferred HTTP route or runtime activation.".into(),
            search_text: vec![page.selection.service.clone(), scope.into(), page.endpoint.symbol.clone()],
            related_links: vec![],
        });
        endpoint.related_links.push(CatalogueLink {
            href: format!("{}-overview.html", page.id),
            title: format!("Selected process: {}", page.id),
        });
        if let Some(question) = &page.selection.question {
            rows.insert(format!("question:{}", page.id), Row {
                id: format!("diagnostic-{}", page.id), title: question.clone(), display_title: None, kind: "Diagnostic question".into(),
                href: format!("{}-diagnostic.html", page.id), context: format!("{} · {scope}", page.selection.service),
                summary: "Supplied diagnostic question; source conditions describe possible reasons, not an approved answer or observed incident.".into(),
                search_text: vec![page.id.clone(), page.selection.service.clone(), scope.into(), page.endpoint.symbol.clone(), page.worker.symbol.clone()],
                related_links: vec![],
            });
        }
    }
    if let Some(graph) = &p.source_call_graph {
        for node in graph.nodes.values() {
            // Selected graph roots can retain only a declaration or interface
            // signature. They remain reachable through process/endpoint pages,
            // but cannot be advertised as an examined source body. A genuine
            // empty method body is available even though it has no steps.
            if node.callable.citation_id.is_none()
                || node.callable.gaps.iter().any(|gap| {
                    matches!(
                        gap.code.as_str(),
                        "CALLABLE_SOURCE_UNAVAILABLE"
                            | "JAVA_PARSE_UNAVAILABLE"
                            | "CALLABLE_BODY_AMBIGUOUS"
                            | "CALLABLE_SYNTAX_PARTIAL"
                            | "CALLABLE_BODY_UNAVAILABLE"
                    )
                })
            {
                continue;
            }
            let related_links = graph
                .reverse_examined_processes
                .get(&node.id)
                .into_iter()
                .flatten()
                .filter(|row| pages.contains_key(row.process_id.as_str()))
                .map(|row| CatalogueLink {
                    href: format!("{}-overview.html", row.process_id),
                    title: format!("Examined by: {}", row.process_id),
                })
                .collect::<Vec<_>>();
            let mut search_text = vec![
                node.service.clone(),
                node.scope.clone(),
                node.callable.symbol.clone(),
            ];
            search_text.extend(
                graph
                    .reverse_examined_processes
                    .get(&node.id)
                    .into_iter()
                    .flatten()
                    .map(|row| row.process_id.clone()),
            );
            rows.insert(node.id.clone(), Row {
                id: node.id.clone(), title: node.callable.symbol.clone(),
                display_title: Some(callable_label(&node.callable.symbol)), kind: "Examined callable".into(),
                href: format!("source-calls.html#{}", anchor(&node.id)),
                context: format!("{} · {}", node.service, node.scope),
                summary: "Retained examined source body; reverse links are documentation context, not runtime impact. Call frontiers remain local gaps.".into(),
                search_text, related_links,
            });
        }
    }
    let mut rows = rows.into_values().collect::<Vec<_>>();
    rows.sort_by(|a, b| (&a.kind, &a.title, &a.id).cmp(&(&b.kind, &b.title, &b.id)));
    Ok(rows)
}

pub(super) fn enhancement(
    rows: &[Row],
    has_declaration_only: bool,
    has_compiler_control_flow: bool,
) -> Result<String, ClewError> {
    let payload = serde_json::to_string(rows)
        .map_err(crate::documentation::io_error)?
        .replace('<', "\\u003c");
    let kinds = rows
        .iter()
        .map(|row| row.kind.as_str())
        .collect::<BTreeSet<_>>();
    let options = kinds
        .into_iter()
        .map(|kind| format!("<option>{}</option>", escape(kind)))
        .collect::<String>();
    let (heading, summary, search_label, noscript) = if has_compiler_control_flow {
        (
            "Find selected declarations and compiler control-flow panels",
            "This snapshot and bundle only. Compiler node IDs are identifiers, not execution order. Graph labels are retained compiler metadata; relationships between selected functions and runtime execution are not inferred.",
            "Search exact symbol, declaration, compiler graph or question",
            "Search requires JavaScript. Use the selected declaration and compiler control-flow links below.",
        )
    } else if has_declaration_only {
        (
            "Find selected declarations and processes",
            "This snapshot and bundle only. Declaration-only pages retain source but do not project behavior or relationships. Reverse links identify documentation context, not runtime impact.",
            "Search exact symbol, declaration, process or question",
            "Search requires JavaScript. Use the selected declaration and process links below.",
        )
    } else {
        (
            "Find selected processes and examined source",
            "This snapshot and bundle only. Diagnostic questions are supplied text, not approved answers. Reverse links identify examined documentation context, not runtime impact.",
            "Search exact symbol, process or question",
            "Search requires JavaScript. Use the process list and shared source links below.",
        )
    };
    Ok(format!(
        r#"<!-- native-catalog-start --><section aria-labelledby="catalog-title"><h2 id="catalog-title">{heading}</h2><p>{summary}</p><div id="catalog-controls" class="catalog-controls" hidden><label>{search_label} <input id="catalog-query" type="search" aria-label="Find native documentation metadata" /></label><label>Result type <select id="catalog-kind" aria-label="Native result type"><option value="">All types</option>{options}</select></label></div><p id="catalog-status" role="status" aria-live="polite"></p><ul id="catalog-results"></ul><div id="catalog-pager" class="catalog-pager" hidden><button id="catalog-prev" type="button">Previous</button><button id="catalog-next" type="button">Next</button></div><noscript><p>{noscript}</p></noscript><script id="catalog-data" type="application/json">{payload}</script></section><!-- native-catalog-end -->"#
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::documentation::model::Observation;
    use serde_json::json;

    fn page(id: &str, scope: &str) -> PageContent {
        // Synthetic publisher fixture, not a compiler evidence qualification.
        let callable = CallableProjection {
            declaration_id: format!("decl-{id}"),
            symbol: "method:class:Same#submit()V".into(),
            authority: "SYNTHETIC_TEST".into(),
            citation_id: None,
            control_flow: None,
            source_outline: None,
            retained_call_sites: None,
            steps: vec![],
            state: vec![],
            gaps: vec![],
        };
        let observation: Observation = serde_json::from_value(json!({"id":callable.declaration_id,"kind":"SYMBOL","service":"service","symbol":callable.symbol,"sourceIds":[],"normalized":{"scope":scope},"digest":"synthetic"})).unwrap();
        PageContent {
            id: id.into(),
            title: id.into(),
            projection_kind: None,
            selection: Selection {
                id: id.into(),
                service: "service".into(),
                endpoint_declaration: callable.declaration_id.clone(),
                worker_declaration: callable.declaration_id.clone(),
                wiring_declaration: None,
                question: Some("Why <script>{x}</script> is delivery absent?".into()),
                note_ids: vec![],
                authored_paragraphs: vec![],
                expand_source_calls: true,
                expand_data_state: false,
            },
            service_revision: "revision".into(),
            service_digest: "digest".into(),
            endpoint: callable.clone(),
            worker: callable,
            wiring: None,
            handoff: HandoffProjection {
                status: "LOCAL_GAP".into(),
                queue_allocation: None,
                endpoint_field: None,
                worker_field: None,
                citation_ids: vec![],
                gaps: vec![],
                limitation: "unverified".into(),
            },
            diagnostics: vec![],
            citations: BTreeMap::new(),
            observations: BTreeMap::from([(observation.id.clone(), observation)]),
            sources: BTreeMap::new(),
            limitations: vec![],
            human_instructions: vec![],
            authored_paragraphs: vec![],
            examined_sources: None,
            data_state: None,
        }
    }
    #[test]
    fn canonical_endpoint_keeps_scope_and_related_processes_without_prose_inference() {
        let mut p = BundleProjection {
            schema: SCHEMA.into(),
            input_digest: "input".into(),
            context_digest: "context".into(),
            selection_digest: "selection".into(),
            endpoint_publication_policy_digest: None,
            requested_selection_digest: None,
            pages: vec![
                page("parent-b", ":/main"),
                page("parent-a", ":/main"),
                page("other", ":/test"),
            ],
            source_call_graph: None,
        };
        let catalogue = rows(&p).unwrap();
        let endpoints = catalogue
            .iter()
            .filter(|row| row.kind == "Endpoint")
            .collect::<Vec<_>>();
        assert_eq!(endpoints.len(), 2);
        let main = endpoints
            .iter()
            .find(|row| row.context.contains(":/main"))
            .unwrap();
        assert_eq!(main.related_links.len(), 2);
        assert_eq!(main.href, "parent-a-endpoint.html");
        assert_eq!(main.title, "method:class:Same#submit()V");
        assert_eq!(main.display_title.as_deref(), Some("Same.submit"));
        assert!(
            catalogue
                .iter()
                .filter(|row| matches!(row.kind.as_str(), "Process" | "Diagnostic question"))
                .all(|row| !serde_json::to_value(row)
                    .unwrap()
                    .as_object()
                    .unwrap()
                    .contains_key("displayTitle"))
        );
        assert_ne!(endpoints[0].id, endpoints[1].id);
        assert_eq!(
            catalogue.iter().filter(|row| row.kind == "Process").count(),
            3
        );
        assert_eq!(
            catalogue
                .iter()
                .filter(|row| row.kind == "Diagnostic question")
                .count(),
            3
        );
        p.pages.reverse();
        p.pages[0].service_revision = "relocated revision".into();
        for page in &mut p.pages {
            let old = page.endpoint.declaration_id.clone();
            let mut observation = page.observations.remove(&old).unwrap();
            observation.id = format!("relocated-{old}");
            page.endpoint.declaration_id = observation.id.clone();
            page.observations
                .insert(observation.id.clone(), observation);
        }
        assert_eq!(rows(&p).unwrap(), catalogue);
        let html = enhancement(&catalogue, false, false).unwrap();
        assert!(!html.contains("<script>{x}"));
        assert!(html.contains("\\u003cscript>"));
        assert!(html.contains("not approved answers") && html.contains("<noscript>"));
    }
    #[test]
    fn selected_unavailable_roots_are_not_examined_body_results_but_empty_bodies_are() {
        let selected = page("bodyless-endpoint", ":/main");
        let make_node = |id: &str, code: Option<&str>, has_source: bool| {
            let mut callable = selected.endpoint.clone();
            callable.citation_id = has_source.then(|| "synthetic-source".into());
            callable.gaps = code
                .into_iter()
                .map(|code| Gap {
                    code: code.into(),
                    detail: "Synthetic body availability fixture".into(),
                    citation_id: callable.citation_id.clone(),
                })
                .collect();
            SourceCallNode {
                id: id.into(),
                service: "service".into(),
                scope: ":/main".into(),
                callable,
                calls: vec![],
                citations: BTreeMap::new(),
                observations: BTreeMap::new(),
                sources: BTreeMap::new(),
                examined_source_digest: "synthetic".into(),
                node_projection_kind: None,
                data_state: None,
            }
        };
        let mut nodes = BTreeMap::new();
        for code in [
            "CALLABLE_BODY_UNAVAILABLE",
            "CALLABLE_BODY_AMBIGUOUS",
            "CALLABLE_SYNTAX_PARTIAL",
            "JAVA_PARSE_UNAVAILABLE",
            "CALLABLE_SOURCE_UNAVAILABLE",
        ] {
            nodes.insert(code.into(), make_node(code, Some(code), true));
        }
        nodes.insert(
            "missing-source".into(),
            make_node("missing-source", None, false),
        );
        nodes.insert("empty-body".into(), make_node("empty-body", None, true));
        let p = BundleProjection {
            schema: SCHEMA.into(),
            input_digest: "input".into(),
            context_digest: "context".into(),
            selection_digest: "selection".into(),
            endpoint_publication_policy_digest: None,
            requested_selection_digest: None,
            pages: vec![selected.clone()],
            source_call_graph: Some(SourceCallGraph {
                schema: "synthetic".into(),
                authority: "SYNTHETIC_TEST".into(),
                max_depth: 2,
                max_additional_bodies: 64,
                max_additional_source_bytes: 1024 * 1024,
                nodes,
                process_links: vec![],
                reverse_examined_processes: BTreeMap::new(),
                reverse_field_references: BTreeMap::new(),
                reverse_property_references: BTreeMap::new(),
            }),
        };
        let catalogue = rows(&p).unwrap();
        let examined = catalogue
            .iter()
            .filter(|row| row.kind == "Examined callable")
            .collect::<Vec<_>>();
        assert_eq!(examined.len(), 1);
        assert_eq!(examined[0].id, "empty-body");
        assert!(
            catalogue
                .iter()
                .any(|row| row.kind == "Process" && row.id == selected.id)
        );
        assert!(
            catalogue
                .iter()
                .any(|row| row.kind == "Endpoint" && row.title == selected.endpoint.symbol)
        );
    }

    #[test]
    fn declaration_only_catalogue_uses_neutral_declaration_and_question_labels() {
        let mut selected = page("declarations", ":/main");
        selected.projection_kind = Some(ProjectionKind::DeclarationOnly);
        selected.title = "Selected functions: render".into();
        selected.selection.question = Some("What source is retained?".into());
        let projection = BundleProjection {
            schema: DECLARATION_SCHEMA.into(),
            input_digest: "input".into(),
            context_digest: "context".into(),
            selection_digest: "selection".into(),
            pages: vec![selected],
            source_call_graph: None,
        };
        let rows = rows(&projection).unwrap();
        assert!(rows.iter().any(|row| row.kind == "Selected declarations"));
        assert!(rows.iter().any(|row| row.kind == "Selected question"));
        assert!(
            rows.iter()
                .all(|row| !matches!(row.kind.as_str(), "Process" | "Endpoint"))
        );
        assert!(
            rows.iter()
                .all(|row| !row.summary.contains("source condition"))
        );
        let html = enhancement(&rows, true, false).unwrap();
        assert!(html.contains("Find selected declarations and processes"));
        assert!(html.contains("do not project behavior or relationships"));
        assert!(!html.contains("Endpoint"));
    }
}
