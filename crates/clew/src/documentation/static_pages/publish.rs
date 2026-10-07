//! A single semantic content tree is emitted as inert JSX and static HTML.
use super::model::*;
#[path = "catalogue.rs"]
mod catalogue;
use crate::documentation::{digest, invalid, io_error, store};
use crate::error::ClewError;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
};

fn escape(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '&' => "&amp;".into(),
            '<' => "&lt;".into(),
            '>' => "&gt;".into(),
            '"' => "&quot;".into(),
            '\'' => "&#39;".into(),
            '@' | '{' | '}' | '\n' | '\r' | '\t' | '*' | '_' | '[' | ']' | '`' | '\\' => {
                format!("&#{};", c as u32)
            }
            _ => c.to_string(),
        })
        .collect()
}
fn anchor(id: &str) -> String {
    format!(
        "ref-{}",
        crate::canonical::hash_bytes(id.as_bytes()).trim_start_matches("sha256:")
    )
}
fn paragraph(text: &str) -> String {
    format!("<p>{}</p>\n", escape(text))
}
fn link(path: &str, label: &str) -> String {
    format!("<a href=\"{}\">{}</a>", escape(path), escape(label))
}
fn cite(id: &str, ext: &str) -> String {
    link(&format!("sources.{ext}#{}", anchor(id)), "Retained source")
}
fn gaps(rows: &[Gap], ext: &str) -> String {
    rows.iter()
        .map(|g| {
            format!(
                "<li>{}: {} {}</li>\n",
                escape(&g.code),
                escape(&g.detail),
                g.citation_id
                    .as_deref()
                    .map(|id| cite(id, ext))
                    .unwrap_or_default()
            )
        })
        .collect::<String>()
}
fn conditions(rows: &[PathCondition], ext: &str) -> String {
    rows.iter()
        .map(|p| {
            format!(
                "<li>{} = {} {}</li>\n",
                escape(&p.expression),
                p.holds,
                cite(&p.citation_id, ext)
            )
        })
        .collect()
}
fn statement(s: &Statement, ext: &str) -> String {
    let mut out = format!(
        "<li><pre>{}</pre><p>Kind: {:?}; reachable in retained source path: {}. {}</p>\n",
        escape(&s.expression),
        s.kind,
        s.reachable,
        cite(&s.citation_id, ext)
    );
    if !s.conditions.is_empty() {
        out += &format!("<ul>{}</ul>\n", conditions(&s.conditions, ext));
    }
    for call in &s.calls {
        out += &paragraph(&format!("Call phase: {}.", call.phase));
        out += &format!("<p>{}</p>\n", cite(&call.citation_id, ext));
        if let Some(node) = &call.expanded_node {
            out += &format!(
                "<p>{}</p>\n",
                link(
                    &format!("source-calls.{ext}#{}", anchor(node)),
                    "Read the retained target body (runtime dispatch unobserved)"
                )
            );
        }
        out += &format!(
            "<details><summary>Exact call identity</summary>{}</details>\n",
            paragraph(&format!(
                "Expression: {}. Target: {}. Authority: {}. Relation: {}.",
                call.expression,
                call.target.as_deref().unwrap_or("unresolved"),
                call.authority,
                call.relation_id.as_deref().unwrap_or("unresolved")
            ))
        );
        if let Some(b) = &call.external_boundary {
            out += &paragraph(&format!(
                "External boundary: {}. Source status: {}. {}",
                b.target, b.source_status, b.limitation
            ));
            for id in &b.citation_ids {
                out += &format!("<p>{}</p>\n", cite(id, ext));
            }
        }
        out += &format!("<ul>{}</ul>\n", gaps(&call.gaps, ext));
    }
    out += &format!("<ul>{}</ul>\n", gaps(&s.gaps, ext));
    if !s.children.is_empty() {
        out += &format!(
            "<p>Body / true alternative</p><ol>{}</ol>\n",
            s.children
                .iter()
                .map(|s| statement(s, ext))
                .collect::<String>()
        );
    }
    if !s.alternative.is_empty() {
        out += &format!(
            "<p>False alternative</p><ol>{}</ol>\n",
            s.alternative
                .iter()
                .map(|s| statement(s, ext))
                .collect::<String>()
        );
    }
    out += "</li>\n";
    out
}
fn callable_label(symbol: &str) -> String {
    symbol
        .strip_prefix("method:class:")
        .or_else(|| symbol.strip_prefix("method:"))
        .unwrap_or(symbol)
        .split('(')
        .next()
        .unwrap_or(symbol)
        .replace('#', ".")
}
fn process_label(p: &PageContent) -> String {
    if p.projection_kind
        .is_some_and(ProjectionKind::is_declaration_view)
    {
        p.title.clone()
    } else {
        format!(
            "{} → {}",
            callable_label(&p.endpoint.symbol),
            callable_label(&p.worker.symbol)
        )
    }
}
fn citation_group(p: &PageContent, ids: &[String], ext: &str) -> String {
    if ids.is_empty() {
        return String::new();
    }
    format!(
        "<p>Sources: {}</p>\n",
        ids.iter()
            .map(|id| {
                let label = p
                    .citations
                    .get(id)
                    .map(|c| {
                        format!(
                            "{}:{}–{}",
                            c.file.rsplit('/').next().unwrap_or(&c.file),
                            c.start_line,
                            c.end_line
                        )
                    })
                    .unwrap_or_else(|| "Source".into());
                link(&format!("sources.{ext}#{}", anchor(id)), &label)
            })
            .collect::<Vec<_>>()
            .join(" · ")
    )
}
fn attempt_summary(callable: &CallableProjection, ext: &str) -> String {
    fn collect<'a>(steps: &'a [Statement], out: &mut Vec<&'a CallProjection>) {
        for step in steps {
            if step.reachable {
                out.extend(&step.calls);
            }
            collect(&step.children, out);
            collect(&step.alternative, out);
        }
    }
    let mut calls = Vec::new();
    collect(&callable.steps, &mut calls);
    calls
        .into_iter()
        .filter(|c| {
            matches!(
                c.phase.as_str(),
                "SUBMISSION_ATTEMPT" | "CONSUMPTION_ATTEMPT" | "DEPENDENCY_CALL_ATTEMPT"
            )
        })
        .map(|c| {
            format!(
                "<li><pre>{}</pre>{}</li>\n",
                escape(&c.expression),
                cite(&c.citation_id, ext)
            )
        })
        .collect()
}
fn callable(c: &CallableProjection, ext: &str) -> String {
    let mut out = paragraph(&callable_label(&c.symbol));
    if let Some(id) = &c.citation_id {
        out += &format!("<p>{}</p>\n", cite(id, ext));
    }
    out += &format!(
        "<ol>{}</ol><ul>{}</ul>\n",
        c.steps
            .iter()
            .map(|s| statement(s, ext))
            .collect::<String>(),
        gaps(&c.gaps, ext)
    );
    out += &format!(
        "<details><summary>Exact declaration identity</summary>{}</details>\n",
        paragraph(&format!(
            "Declaration: {}. Symbol: {}. Authority: {}.",
            c.declaration_id, c.symbol, c.authority
        ))
    );
    out
}
fn state(c: &CallableProjection, ext: &str) -> String {
    let mut out = format!(
        "<h2>{}</h2><table tabIndex=\"0\"><caption>Scroll horizontally: fields and state</caption><thead><tr><th scope=\"col\">Name</th><th scope=\"col\">Expression</th><th scope=\"col\">Kind</th><th scope=\"col\">Condition path</th><th scope=\"col\">Source</th></tr></thead><tbody>\n",
        escape(&callable_label(&c.symbol))
    );
    for row in &c.state {
        out += &format!(
            "<tr><td>{}</td><td><pre>{}</pre></td><td>{}</td><td><ul>{}</ul></td><td>{}</td></tr>\n",
            escape(&row.name),
            escape(&row.expression),
            escape(&row.kind),
            conditions(&row.conditions, ext),
            cite(&row.citation_id, ext)
        );
    }
    out += "</tbody></table>\n";
    out
}
fn operational_instructions(rows: &[HumanInstruction]) -> String {
    let mut out =
        "<section id=\"operational-instructions\"><h2>Operational instructions</h2>\n".to_string();
    out += &paragraph(
        "Captured human/imported instructions are unverified. Source-claim status: UNASSESSED; their content does not establish source or runtime behavior.",
    );
    for note in rows {
        out += &format!("<article><h3>{}</h3>\n", escape(&note.title));
        out += &paragraph(&format!(
            "Declared author: {}. Classification: {}. Period: {}. Authority: {}. Source-claim status: {}.",
            note.declared_author,
            note.classification,
            note.period,
            note.authority,
            note.source_claim_status
        ));
        out += &format!("<pre>{}</pre>\n", escape(&note.text));
        out += &format!(
            "<details><summary>Captured note identity</summary>{}</details></article>\n",
            paragraph(&format!(
                "Note ID: {}. Version digest: {}. Content digest: {}. Association digest: {}.",
                note.id, note.version_digest, note.content_digest, note.association_digest
            ))
        );
    }
    out += "</section>\n";
    out
}

const VIEWS: &[(&str, &str)] = &[
    ("overview", "Process overview"),
    ("endpoint", "Endpoint path"),
    ("worker", "Worker path"),
    ("fields-state", "Fields and state"),
    ("diagnostic", "Diagnostic matrix"),
];
const DECLARATION_VIEWS: &[(&str, &str)] = &[
    ("overview", "Selected declarations"),
    ("endpoint", "First selected declaration"),
    ("worker", "Second selected declaration"),
    ("fields-state", "Declaration sources"),
    ("diagnostic", "Question and limits"),
];

fn views_for(p: &PageContent) -> &'static [(&'static str, &'static str)] {
    if p.projection_kind
        .is_some_and(ProjectionKind::is_declaration_view)
    {
        DECLARATION_VIEWS
    } else {
        VIEWS
    }
}

fn control_flow_panel(c: &CallableProjection, ext: &str, full: bool) -> String {
    let Some(graph) = &c.control_flow else {
        return String::new();
    };
    let mut out = format!(
        "<section><h3>Compiler-provided local control flow</h3>\n{}",
        paragraph(&format!(
            "Provider: {}. Nodes: {}. Edges: {}. Node IDs are identifiers, not execution order. Edge kinds and labels are retained compiler metadata; labels are not interpreted as conditions or branch truth.",
            graph.provider,
            graph.nodes.len(),
            graph.edges.len()
        ))
    );
    out += &format!(
        "<details><summary>Compiler graph bindings</summary>{}</details>\n",
        paragraph(&format!(
            "Graph ID: {}. Graph observation: {}. Graph evidence binding: {}. Descriptor evidence binding: {}. Compiler graph name: {}.",
            graph.graph_id,
            graph.graph_observation_id,
            graph.graph_evidence_binding,
            graph.descriptor_evidence_binding,
            graph.compiler_graph_name
        ))
    );
    if full {
        out += "<table><caption>Retained compiler control-flow nodes and outgoing edges</caption><thead><tr><th scope=\"col\">Node ID</th><th scope=\"col\">Role</th><th scope=\"col\">Source</th><th scope=\"col\">Outgoing target, kind and label</th></tr></thead><tbody>\n";
        for node in &graph.nodes {
            let source = node
                .citation_id
                .as_deref()
                .map(|id| cite(id, ext))
                .unwrap_or_else(|| "No source range retained for this node.".into());
            let outgoing = graph
                .edges
                .iter()
                .filter(|edge| edge.source_node_id == node.node_id)
                .map(|edge| {
                    format!(
                        "<li>Target node ID {} · kind {} · label {}</li>\n",
                        edge.target_node_id,
                        control_flow_edge_label(edge.kind),
                        edge.label
                            .as_deref()
                            .map(escape)
                            .unwrap_or_else(|| "(none)".into())
                    )
                })
                .collect::<String>();
            out += &format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td><td><ul>{}</ul></td></tr>\n",
                node.node_id,
                control_flow_role_label(node.role),
                source,
                outgoing
            );
        }
        out += "</tbody></table>\n";
    }
    out += "</section>\n";
    out
}

/// Common inert rendering for exact source coordinates admitted by any adapter.
fn exact_source_span_item(exact: &crate::documentation::source_span::ExactSourceSpan) -> String {
    format!(
        "<li>Retained source: <code>{}</code></li>\n",
        escape(&exact.expression)
    )
}

fn source_outline_panel(c: &CallableProjection, ext: &str) -> String {
    let Some(outline) = &c.source_outline else {
        return String::new();
    };
    let kotlin = outline.authority == "KOTLIN_PSI_WITH_K2_CALL_TARGETS";
    let label = match outline.authority.as_str() {
        "KOTLIN_PSI_WITH_K2_CALL_TARGETS" => "Kotlin",
        "TYPESCRIPT_COMPILER_SOURCE_STRUCTURE" => "TypeScript",
        "RUST_SYN_SOURCE_STRUCTURE" => "Rust",
        _ => "C#",
    };
    let event_label = if kotlin { "PSI" } else { "Source" };
    let explanation = if kotlin {
        "This tree preserves retained Kotlin PSI structure and exact K2 call-target metadata. It is not an execution trace: PSI conditions do not establish predicate truth, and event ordinals do not represent runtime invocation counts."
    } else if outline.authority == "TYPESCRIPT_COMPILER_SOURCE_STRUCTURE" {
        "This tree preserves retained TypeScript compiler source structure and admitted selected-declaration targets. Source conditions do not establish predicate truth; event ordinals do not represent runtime invocation counts, and receiver dispatch remains unresolved."
    } else if outline.authority == "RUST_SYN_SOURCE_STRUCTURE" {
        "This tree preserves exact retained Rust syntax. Calls remain unresolved source text; parsing does not establish compiler type resolution, predicate truth, receiver identity, reachability or state effects."
    } else {
        "This tree preserves retained Roslyn source structure and exact compiler call-target metadata. It is not an execution trace: source conditions do not establish predicate truth, and event ordinals do not represent runtime invocation counts."
    };
    let mut out = format!(
        "<section><h3>Cited {label} source outline</h3>\n{}",
        paragraph(explanation)
    );
    if let Some(tree) = &outline.tree {
        out += &format!("<pre>{}</pre>\n", escape(tree));
    } else {
        out += &format!("<ul>{}</ul>\n", gaps(&outline.gaps, ext));
    }
    if !outline.events.is_empty() {
        out += &format!("<ul aria-label=\"Retained {event_label} event citations\">\n");
        for event in &outline.events {
            out += &format!(
                "<li>{event_label} event ordinal {} · {} · {}-{}: {}</li>\n",
                event.ordinal,
                escape(&event.kind),
                event.start_line,
                event.end_line,
                cite(&event.citation_id, ext)
            );
            if let Some(exact) = &event.exact_source {
                out += &exact_source_span_item(exact);
            }
            if !event.gaps.is_empty() {
                out += &format!("<li><ul>{}</ul></li>\n", gaps(&event.gaps, ext));
            }
        }
        out += "</ul>\n";
    }
    out += "</section>\n";
    out
}

fn retained_call_sites_panel(
    c: &CallableProjection,
    ext: &str,
    service: &str,
    graph: Option<&SourceCallGraph>,
) -> String {
    let Some(retained) = &c.retained_call_sites else {
        return String::new();
    };
    let source_event = |id: &str| {
        c.source_outline.as_ref().is_some_and(|outline| {
            outline
                .events
                .iter()
                .any(|event| event.observation_id == id)
        })
    };
    let has_source_events = retained
        .sites
        .iter()
        .any(|site| source_event(&site.relation_id));
    let mut out = format!(
        "<section><h3>Retained exact call sites</h3>\n{}",
        paragraph(if has_source_events {
            "These sites come from retained compiler call relations or compiler-target source events with exact captured spans. Source-event sites do not supply receiver identity or argument mappings. No site establishes runtime execution, invocation count, ordering, or reachability."
        } else {
            "These are retained compiler call relations with their captured source snippets. They do not establish runtime execution, invocation count, ordering, or reachability."
        })
    );
    if retained.sites.is_empty() {
        out += &format!("<ul>{}</ul>\n", gaps(&retained.gaps, ext));
    } else {
        let owner_node = graph.and_then(|graph| {
            graph.nodes.values().find(|node| {
                node.service == service
                    && node.callable.declaration_id == c.declaration_id
                    && node.callable.symbol == c.symbol
                    && node.node_projection_kind.is_some()
            })
        });
        out += "<ol aria-label=\"Retained exact call sites\">\n";
        for site in &retained.sites {
            out += &format!(
                "<li><p>Target identity: {}. Source: {}:{}-{}. Captured source span bytes [{}, {}). {}: {}. {}",
                escape(&site.target_identity),
                escape(&site.file),
                site.start_line,
                site.end_line,
                site.compilation_byte_start,
                site.compilation_byte_end,
                if source_event(&site.relation_id) {
                    "Compiler source event"
                } else {
                    "Relation"
                },
                escape(&site.relation_id),
                cite(&site.citation_id, ext)
            );
            if let Some(edge) = owner_node.and_then(|node| {
                node.calls.iter().find(|edge| {
                    edge.exact_call_site
                        .as_ref()
                        .is_some_and(|candidate| candidate.relation_id == site.relation_id)
                })
            }) {
                if let Some(target_node) = &edge.target_node {
                    out += &format!(
                        " Navigation: {}.",
                        link(
                            &format!("source-calls.{ext}#{}", anchor(target_node)),
                            "Retained target body"
                        )
                    );
                } else {
                    let frontier_codes = edge
                        .frontiers
                        .iter()
                        .map(|gap| gap.code.as_str())
                        .collect::<Vec<_>>()
                        .join(", ");
                    out += &format!(
                        " Navigation status: {}. Target declaration: {}. Frontiers: {}.",
                        escape(&edge.status),
                        escape(edge.target_declaration.as_deref().unwrap_or("unavailable")),
                        escape(if frontier_codes.is_empty() {
                            "none"
                        } else {
                            &frontier_codes
                        })
                    );
                }
            }
            out += &format!("</p><pre>{}</pre>", escape(&site.expression));
            out += &argument_bindings_panel(site, ext);
            out += "</li>\n";
        }
        out += "</ol>\n";
    }
    out += "</section>\n";
    out
}

fn argument_bindings_panel(site: &NeutralExactCallSite, ext: &str) -> String {
    let Some(bindings) = &site.argument_bindings else {
        return String::new();
    };
    let mut out = String::from("<div><h4>Compiler argument bindings</h4>\n");
    if !bindings.gaps.is_empty() {
        out += &format!("<ul>{}</ul>\n", gaps(&bindings.gaps, ext));
        out += "</div>\n";
        return out;
    }
    if bindings.arguments.is_empty() {
        out += "<p>No explicit argument mappings were reported by the retained compiler payload.</p>\n";
    } else {
        out += "<ul aria-label=\"Compiler argument-to-parameter mappings\">\n";
        for argument in &bindings.arguments {
            let named = argument
                .argument_name
                .as_deref()
                .map(|name| format!(" Named argument: {}.", escape(name)))
                .unwrap_or_default();
            out += &format!(
                "<li>Captured argument bytes [{}, {}).{} Argument type: {}. Formal parameter {} (index {}): {}. {}<pre>{}</pre></li>\n",
                argument.compilation_byte_start,
                argument.compilation_byte_end,
                named,
                escape(&argument.argument_type),
                escape(&argument.parameter),
                argument.parameter_index,
                escape(&argument.parameter_type),
                cite(&argument.citation_id, ext),
                escape(&argument.expression)
            );
        }
        out += "</ul>\n";
    }
    if !bindings.omitted_default_parameter_indices.is_empty() {
        let indices = bindings
            .omitted_default_parameter_indices
            .iter()
            .map(u64::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        out += &format!(
            "<p>Omitted default parameter indices reported by the compiler: {}.</p>\n",
            escape(&indices)
        );
    }
    out += "</div>\n";
    out
}

#[allow(clippy::too_many_arguments)]
fn selected_declaration(
    c: &CallableProjection,
    label: &str,
    ext: &str,
    full_control_flow: bool,
    service: &str,
    graph: Option<&SourceCallGraph>,
    observations: &BTreeMap<String, crate::documentation::model::Observation>,
    sources: &BTreeMap<String, crate::documentation::model::Source>,
) -> String {
    let mut out = format!("<section><h2>{}</h2>\n", escape(label));
    out += &paragraph(&format!(
        "Declaration: {}. Symbol: {}. Authority: {}.",
        c.declaration_id, c.symbol, c.authority
    ));
    if let Some(id) = &c.citation_id {
        out += &format!("<p>Retained source: {}</p>\n", cite(id, ext));
    }
    if let Some(signature) = observations
        .get(&c.declaration_id)
        .and_then(|owner| super::source::compiler::declared_signature(owner, sources))
    {
        let function = observations.get(&c.declaration_id).is_some_and(|owner| {
            matches!(
                owner.normalized["declarationKind"].as_str(),
                Some("FUNCTION" | "METHOD" | "function" | "impl-method" | "trait-method")
            )
        });
        let label = if function {
            "Retained input and return signature"
        } else {
            "Retained declaration source"
        };
        out += &format!("<p>{label}:</p><pre>{}</pre>\n", escape(signature));
    }
    out += &control_flow_panel(c, ext, full_control_flow);
    out += &source_outline_panel(c, ext);
    out += &retained_call_sites_panel(c, ext, service, graph);
    if !c.gaps.is_empty() {
        out += &format!("<ul>{}</ul>\n", gaps(&c.gaps, ext));
    }
    out += "</section>\n";
    out
}

fn control_flow_role_label(role: crate::thread_flow_cfg::LocalCfgNodeRole) -> &'static str {
    use crate::thread_flow_cfg::LocalCfgNodeRole::*;
    match role {
        Entry => "ENTRY",
        Exit => "EXIT",
        Operation => "OPERATION",
        Decision => "DECISION",
        Merge => "MERGE",
        Return => "RETURN",
        Throw => "THROW",
        Catch => "CATCH",
        Finally => "FINALLY",
        LoopCondition => "LOOP_CONDITION",
        LoopExit => "LOOP_EXIT",
        Dead => "DEAD",
    }
}

fn control_flow_edge_label(kind: crate::thread_flow_cfg::LocalCfgEdgeKind) -> &'static str {
    use crate::thread_flow_cfg::LocalCfgEdgeKind::*;
    match kind {
        Next => "NEXT",
        True => "TRUE",
        False => "FALSE",
        WhenCase => "WHEN_CASE",
        Exception => "EXCEPTION",
        Return => "RETURN",
        LoopBack => "LOOP_BACK",
        Break => "BREAK",
        Continue => "CONTINUE",
        Finally => "FINALLY",
        Dead => "DEAD",
    }
}

fn declaration_only_page(
    p: &PageContent,
    view: &str,
    title: &str,
    snapshot: &str,
    ext: &str,
    graph: Option<&SourceCallGraph>,
) -> String {
    let graph = p.selection.expand_source_calls.then_some(graph).flatten();
    let same_declaration = p.endpoint.declaration_id == p.worker.declaration_id;
    let has_control_flow = p.projection_kind == Some(ProjectionKind::CompilerControlFlow);
    let mut out = format!(
        "<main><h1>{}: {}</h1>\n<nav aria-label=\"Documentation pages\">{}",
        escape(&p.title),
        escape(title),
        link(&format!("index.{ext}"), "All selected declarations")
    );
    for (slug, label) in DECLARATION_VIEWS {
        out += &format!(" · {}", link(&format!("{}-{slug}.{ext}", p.id), label));
    }
    out += &format!(
        " · {}</nav>\n",
        link(&format!("sources.{ext}"), "Source appendix")
    );
    match view {
        "overview" => {
            if let Some(question) = &p.selection.question {
                out += &paragraph(&format!("Question: {question}"));
            }
            out += &paragraph(if has_control_flow {
                "This page retains selected compiler declarations and function-local compiler control-flow graphs. The graphs do not establish relationships between declarations, execution order, branch truth or deployed runtime behavior."
            } else if p.endpoint.authority == "SYNTAX_DECLARATION"
                || p.worker.authority == "SYNTAX_DECLARATION"
            {
                "This page retains selected syntax declarations, source signatures and cited source structure. Parsing does not establish compiler resolution, runtime behavior or relationships between declarations."
            } else {
                "This page retains selected compiler declarations and their source. It does not project source behavior or infer relationships between declarations."
            });
            if same_declaration {
                out += &selected_declaration(
                    &p.endpoint,
                    "Selected declaration",
                    ext,
                    false,
                    &p.selection.service,
                    graph,
                    &p.observations,
                    &p.sources,
                );
            } else {
                out += &selected_declaration(
                    &p.endpoint,
                    "First selected declaration",
                    ext,
                    false,
                    &p.selection.service,
                    graph,
                    &p.observations,
                    &p.sources,
                );
                out += &selected_declaration(
                    &p.worker,
                    "Second selected declaration",
                    ext,
                    false,
                    &p.selection.service,
                    graph,
                    &p.observations,
                    &p.sources,
                );
            }
            if let Some(wiring) = &p.wiring {
                out += &selected_declaration(
                    wiring,
                    "Additional selected declaration",
                    ext,
                    false,
                    &p.selection.service,
                    graph,
                    &p.observations,
                    &p.sources,
                );
            }
            out += &paragraph(&p.handoff.limitation);
        }
        "endpoint" => {
            out += &selected_declaration(
                &p.endpoint,
                if same_declaration {
                    "Selected declaration"
                } else {
                    "First selected declaration"
                },
                ext,
                true,
                &p.selection.service,
                graph,
                &p.observations,
                &p.sources,
            );
        }
        "worker" => {
            out += &selected_declaration(
                &p.worker,
                if same_declaration {
                    "Selected declaration"
                } else {
                    "Second selected declaration"
                },
                ext,
                true,
                &p.selection.service,
                graph,
                &p.observations,
                &p.sources,
            );
        }
        "fields-state" => {
            out += &paragraph(
                "Field and data-state analysis is unavailable for declaration-only pages.",
            );
            out += &selected_declaration(
                &p.endpoint,
                "First retained declaration",
                ext,
                false,
                &p.selection.service,
                graph,
                &p.observations,
                &p.sources,
            );
            if !same_declaration {
                out += &selected_declaration(
                    &p.worker,
                    "Second retained declaration",
                    ext,
                    false,
                    &p.selection.service,
                    graph,
                    &p.observations,
                    &p.sources,
                );
            }
            if let Some(wiring) = &p.wiring {
                out += &selected_declaration(
                    wiring,
                    "Additional retained declaration",
                    ext,
                    true,
                    &p.selection.service,
                    graph,
                    &p.observations,
                    &p.sources,
                );
            }
        }
        "diagnostic" => {
            if let Some(question) = &p.selection.question {
                out += &paragraph(&format!("Question: {question}"));
            }
            out += &paragraph(
                "Source-condition analysis is unavailable for declaration-only pages. The selected declarations do not establish behavior or a relationship between them.",
            );
        }
        _ => unreachable!(),
    }
    if !p.human_instructions.is_empty() {
        if view == "overview" {
            out += &operational_instructions(&p.human_instructions);
        } else {
            out += &format!(
                "<p>{}</p>\n",
                link(
                    &format!("{}-overview.{ext}#operational-instructions", p.id),
                    "Operational instructions and captured note identities"
                )
            );
        }
    }
    if !p.authored_paragraphs.is_empty() {
        if view == "overview" {
            out += &authored_paragraphs(&p.authored_paragraphs, ext);
        } else {
            out += &format!(
                "<p>{}</p>\n",
                link(
                    &format!("{}-overview.{ext}#authored-paragraphs", p.id),
                    "Maintained user documentation and original source context"
                )
            );
        }
    }
    if p.selection.expand_source_calls && graph.is_some() {
        out += &format!(
            "<p>{}</p>\n",
            link(
                &format!("source-calls.{ext}"),
                "Expanded exact source-call context"
            )
        );
    }
    out += &format!(
        "<details><summary>Sources and version</summary>{}</details>\n",
        paragraph(&format!(
            "Service: {}. Revision: {}. Snapshot: {}. Service digest: {}. First selected symbol: {}. Second selected symbol: {}.",
            p.selection.service,
            p.service_revision,
            snapshot,
            p.service_digest,
            p.endpoint.symbol,
            p.worker.symbol
        ))
    );
    out += &format!(
        "<details><summary>Retained limitations</summary><ul>{}</ul></details><p>{}</p></main>\n",
        gaps(&p.limitations, ext),
        link(
            "projection.json",
            "Complete typed projection and retained evidence"
        )
    );
    out
}
fn authored_source_anchor(paragraph: &AuthoredParagraph, id: &str) -> String {
    // Presentation anchors name the existing frozen tuple, not a new SOURCE ID.
    anchor(
        &serde_json::json!([
            "authored-source",
            paragraph.selection,
            paragraph.paragraph_digest,
            id
        ])
        .to_string(),
    )
}
fn authored_paragraphs(rows: &[AuthoredParagraph], ext: &str) -> String {
    let mut out =
        "<section id=\"authored-paragraphs\"><h2>Maintained user documentation</h2>\n".to_owned();
    for row in rows {
        let authorship = row.paragraph.authorship.as_ref().unwrap();
        out += &format!("<article><pre>{}</pre>\n", escape(&row.paragraph.text));
        if let Some(migration) = &authorship.context_migration {
            out += &paragraph(&format!(
                "Declared text author: {}. Authority: USER_DOCUMENTATION. Meaning review: UNASSESSED. Context selected by {}. Context review: UNASSESSED. Explicitly selected code is unverified context. Source context freshness: {}. Current freshness does not establish semantic review.",
                authorship.author, migration.editor, row.context_freshness
            ));
            out += &paragraph(&format!(
                "Previous source snapshot: {}. Previous context digest: {}. Context instruction digest: {}.",
                migration.previous_source_snapshot,
                migration.previous_context_digest,
                migration.instruction_digest
            ));
        } else {
            out += &paragraph(&format!(
                "Declared author: {}. Authority: USER_DOCUMENTATION. Meaning review: UNASSESSED. Originally linked code is retained unverified context. Source context freshness: {}.",
                authorship.author, row.context_freshness
            ));
        }
        out += "<ul>\n";
        for id in &row.paragraph.source_ids {
            out += &format!(
                "<li>{}</li>\n",
                link(
                    &format!("sources.{ext}#{}", authored_source_anchor(row, id)),
                    &format!(
                        "{} source {id}",
                        if authorship.context_migration.is_some() {
                            "Explicitly selected"
                        } else {
                            "Originally linked"
                        }
                    )
                )
            );
        }
        out += "</ul><details><summary>Frozen paragraph identity</summary>\n";
        out += &paragraph(&format!(
            "Bundle: {}. Operation: {}. Fragment: {}. Paragraph digest: {}. Operation digest: {}. Bindings digest: {}. Publication digest: {}. {} source snapshot: {}. Context role: RETAINED_UNVERIFIED_CONTEXT.",
            row.selection.bundle,
            row.selection.operation,
            row.selection.fragment,
            row.paragraph_digest,
            row.operation_digest,
            row.bindings_digest,
            row.publication_digest,
            if authorship.context_migration.is_some() {
                "Explicitly selected"
            } else {
                "Original"
            },
            authorship.source_snapshot
        ));
        out += "</details></article>\n";
    }
    out += "</section>\n";
    out
}
fn process_calls(p: &PageContent, graph: &SourceCallGraph, ext: &str) -> String {
    let mut out = "<section id=\"source-call-navigation\"><h2>Linked process source context</h2>\n"
        .to_owned();
    out += &paragraph(
        "Links identify retained source calls to selected endpoints. Child workers are selected context; receiver identity, scheduling, runtime dispatch and delivery are unobserved.",
    );
    for relation in graph
        .process_links
        .iter()
        .filter(|l| l.from_process == p.id)
    {
        let node = &graph.nodes[&relation.caller_node];
        let edge = node
            .calls
            .iter()
            .find(|e| e.occurrence_path.as_deref() == Some(relation.occurrence_path.as_str()))
            .unwrap();
        let call = edge
            .call
            .as_ref()
            .expect("process links are Java call edges");
        let occurrence = edge
            .occurrence_path
            .as_deref()
            .expect("process links have Java occurrence paths");
        let path_conditions = edge.conditions.as_deref().unwrap_or_default();
        out += &format!(
            "<article><h3>{}</h3><pre>{}</pre><p>{} · {}</p><ul>{}</ul>\n",
            link(
                &format!("{}-overview.{ext}", relation.to_process),
                &format!("Selected process {}", relation.to_process)
            ),
            escape(&call.expression),
            cite(&relation.citation_id, ext),
            link(
                &format!("source-calls.{ext}#{}", anchor(&relation.caller_node)),
                "Caller body and exact occurrence"
            ),
            conditions(path_conditions, ext)
        );
        out += &paragraph(&format!(
            "Arguments: {}. Target: {}. Scope: {}. Source occurrence: {}. Structurally reachable: {}. Receiver lineage: {}. Runtime dispatch: {}.",
            call.arguments.join(", "),
            call.target.as_deref().unwrap_or("unresolved"),
            edge.target_scope,
            occurrence,
            edge.reachable.unwrap_or(false),
            edge.receiver_lineage,
            edge.runtime_dispatch
        ));
        out += &paragraph(&relation.limitation);
        out += "</article>\n";
    }
    for relation in graph.process_links.iter().filter(|l| l.to_process == p.id) {
        out += &format!(
            "<p>Source caller: {} · {}</p>\n",
            link(
                &format!("{}-worker.{ext}", relation.from_process),
                &relation.from_process
            ),
            cite(&relation.citation_id, ext)
        );
    }
    if let Some(examined) = &p.examined_sources {
        out += &paragraph(&format!(
            "Examined source digest: {}. Schema: {}. This is a documentation review fingerprint, not a file digest or runtime impact claim.",
            examined.examined_source_digest, examined.schema
        ));
        out += &format!(
            "<p>{}</p>\n",
            link(
                &format!("source-calls.{ext}"),
                "Expanded source bodies, frontiers and reverse examined context"
            )
        );
    }
    if let Some(state) = &p.data_state {
        out += &paragraph(&format!(
            "Source data state digest: {}. This separate syntax fingerprint does not prove runtime values or completion.",
            state.data_state_digest
        ));
    }
    out += "</section>\n";
    out
}

fn expanded_sources(graph: &SourceCallGraph, ext: &str) -> String {
    let mut out = "<main><h1>Retained source-call bodies</h1>\n".to_owned();
    out += &format!(
        "<p>{} · {}</p>\n",
        link(&format!("index.{ext}"), "All processes"),
        link(&format!("sources.{ext}"), "Exact retained source appendix")
    );
    out += &paragraph(&format!(
        "Expansion: at most {} source-call edges from a selected root, {} additional bodies and {} additional retained-source bytes. Cached body links and frontiers do not prove execution or instance lineage.",
        graph.max_depth, graph.max_additional_bodies, graph.max_additional_source_bytes
    ));
    for node in graph.nodes.values() {
        out += &format!(
            "<section id=\"{}\"><h2>{}</h2>\n",
            anchor(&node.id),
            escape(&callable_label(&node.callable.symbol))
        );
        out += &paragraph(&format!(
            "Service: {}. Compiler scope: {}. Examined source digest: {}.",
            node.service, node.scope, node.examined_source_digest
        ));
        let producer_label = if node
            .observations
            .get(&node.callable.declaration_id)
            .is_some_and(|owner| {
                owner.normalized["schema"] == crate::csharp_adapter_v2::CSHARP_FACT_SCHEMA
            }) {
            "C#"
        } else if node
            .observations
            .get(&node.callable.declaration_id)
            .is_some_and(|owner| {
                owner.normalized["schema"] == crate::typescript_adapter_v2::TYPESCRIPT_FACT_SCHEMA
            })
        {
            "TypeScript"
        } else {
            "Kotlin"
        };
        if node.node_projection_kind.is_some() {
            out += &selected_declaration(
                &node.callable,
                &format!("Retained {producer_label} declaration"),
                ext,
                true,
                &node.service,
                Some(graph),
                &node.observations,
                &node.sources,
            );
        } else {
            out += &callable(&node.callable, ext);
        }
        if let Some(state) = &node.data_state {
            out += &format!(
                "<details id=\"{}-data\"><summary>Source data transformations</summary><h3>Guarded definitions and call prerequisites</h3>",
                anchor(&node.id)
            );
            out += &paragraph(&format!(
                "Data state digest: {}. Compiler facts identify declarations; transformations and guards describe source syntax. Values, receiver aliases, external results and normal runtime completion are unproven.",
                state.data_state_digest
            ));
            for definition in &state.definitions {
                out += &format!(
                    "<details><summary>{}</summary><pre>{}</pre>{}</details>",
                    escape(&definition.id),
                    escape(&serde_json::to_string_pretty(definition).unwrap()),
                    definition
                        .citation_id
                        .as_ref()
                        .map(|c| cite(c, ext))
                        .unwrap_or_default()
                );
            }
            out += "<h3>Per-occurrence argument and return mappings</h3>";
            out += &format!(
                "<pre>{}</pre><ul>{}</ul></details>",
                escape(&serde_json::to_string_pretty(&state.calls).unwrap()),
                gaps(&state.gaps, ext)
            );
        }
        for edge in &node.calls {
            if let Some(call) = &edge.call {
                out += &format!(
                    "<details><summary>Source call {}</summary><pre>{}</pre><ul>{}</ul>\n",
                    escape(edge.occurrence_path.as_deref().unwrap_or("")),
                    escape(&call.expression),
                    conditions(edge.conditions.as_deref().unwrap_or_default(), ext)
                );
                out += &paragraph(&format!(
                    "Status: {}. Source identity: {}. Target scope: {}. Target declaration: {}. Relation digest: {}. Receiver lineage: {}. Runtime dispatch: {}.",
                    edge.status,
                    edge.source_identity,
                    edge.target_scope,
                    edge.target_declaration.as_deref().unwrap_or("unavailable"),
                    edge.relation_digest.as_deref().unwrap_or("unavailable"),
                    edge.receiver_lineage,
                    edge.runtime_dispatch
                ));
            } else if let Some(site) = &edge.exact_call_site {
                out += &format!(
                    "<details><summary>Exact {producer_label} source site {}</summary><pre>{}</pre>",
                    escape(&site.relation_id),
                    escape(&site.expression)
                );
                out += &format!(
                    "<p>Status: {}. Source identity: {}. Target scope: {}. Target declaration: {}. Normalized relation digest: {}. Captured bytes [{}, {}). {}</p>\n",
                    escape(&edge.status),
                    escape(&edge.source_identity),
                    escape(&edge.target_scope),
                    escape(edge.target_declaration.as_deref().unwrap_or("unavailable")),
                    escape(&site.normalized_digest),
                    site.compilation_byte_start,
                    site.compilation_byte_end,
                    cite(&site.citation_id, ext)
                );
                out += &argument_bindings_panel(site, ext);
                if let Some(target) = &edge.target_node {
                    out += &format!(
                        "<p>{}</p>\n",
                        link(
                            &format!("source-calls.{ext}#{}", anchor(target)),
                            "Cached target declaration",
                        )
                    );
                }
                out += &format!(
                    "<p>Receiver lineage: {}. Runtime dispatch: {}.</p>\n",
                    escape(&edge.receiver_lineage),
                    escape(&edge.runtime_dispatch)
                );
            }
            out += &format!("<ul>{}</ul></details>\n", gaps(&edge.frontiers, ext));
        }
        out += "<h3>Examined documentation context</h3><ul>\n";
        if let Some(rows) = graph.reverse_examined_processes.get(&node.id) {
            for row in rows {
                out += &format!(
                    "<li>{}: {}</li>\n",
                    link(
                        &format!("{}-overview.{ext}", row.process_id),
                        &row.process_id
                    ),
                    escape(
                        &row.reasons
                            .iter()
                            .map(|r| format!(
                                "{}{}",
                                r.reason,
                                r.via_process
                                    .as_ref()
                                    .map(|p| format!(" via {p}"))
                                    .unwrap_or_default()
                            ))
                            .collect::<Vec<_>>()
                            .join("; ")
                    )
                );
            }
        }
        out += "</ul></section>\n";
    }
    if !graph.reverse_field_references.is_empty() {
        out += "<h2>Examined field declaration references</h2>";
        out += &paragraph(
            "These are references in examined source bodies, not runtime instance identities or an impact inventory.",
        );
        for (field, nodes) in &graph.reverse_field_references {
            out += &format!(
                "<p>{}: {}</p>",
                escape(field),
                nodes
                    .iter()
                    .map(|node| link(&format!("source-calls.{ext}#{}-data", anchor(node)), node))
                    .collect::<Vec<_>>()
                    .join(" · ")
            );
        }
    }
    out += "</main>\n";
    out
}

fn page(
    p: &PageContent,
    view: &str,
    title: &str,
    snapshot: &str,
    ext: &str,
    graph: Option<&SourceCallGraph>,
) -> String {
    if p.projection_kind
        .is_some_and(ProjectionKind::is_declaration_view)
    {
        return declaration_only_page(p, view, title, snapshot, ext, graph);
    }
    let mut out = format!(
        "<main><h1>{}: {}</h1>\n<nav aria-label=\"Documentation pages\">{}",
        escape(&process_label(p)),
        escape(title),
        link(&format!("index.{ext}"), "All processes")
    );
    for (slug, label) in views_for(p) {
        out += &format!(" · {}", link(&format!("{}-{slug}.{ext}", p.id), label));
    }
    out += &format!(
        " · {}</nav>\n",
        link(&format!("sources.{ext}"), "Source appendix")
    );
    match view {
        "overview" => {
            if p.handoff.status == "SOURCE_DECLARED_SHARED_QUEUE" {
                out += &paragraph(&format!(
                    "Source binds {} and {} to the same queue allocation.",
                    p.handoff
                        .endpoint_field
                        .as_deref()
                        .unwrap_or("endpoint field"),
                    p.handoff.worker_field.as_deref().unwrap_or("worker field")
                ));
            } else {
                out += &paragraph("Shared queue wiring unresolved.");
                out += &format!("<ul>{}</ul>\n", gaps(&p.handoff.gaps, ext));
            }
            out += &paragraph(&p.handoff.limitation);
            out += &citation_group(p, &p.handoff.citation_ids, ext);
            out += "<h2>Source call attempts</h2><ul>\n";
            out += &attempt_summary(&p.endpoint, ext);
            out += &attempt_summary(&p.worker, ext);
            out += "</ul>\n";
            let mut seen = BTreeSet::new();
            let guards: Vec<_> = p
                .diagnostics
                .iter()
                .filter(|d| seen.insert(&d.condition))
                .take(4)
                .collect();
            if !guards.is_empty() {
                out += "<h2>Worker guard examples</h2><ul>\n";
                for diagnostic in guards {
                    out += &format!("<li>{}</li>\n", escape(&diagnostic.condition));
                }
                out += &format!(
                    "</ul><p>{}</p>\n",
                    link(
                        &format!("{}-diagnostic.{ext}", p.id),
                        "All diagnostic conditions and inspection points"
                    )
                );
            }
            out += &format!(
                "<details><summary>Exact handoff identity</summary>{}</details>\n",
                paragraph(&format!(
                    "Handoff status: {}. Queue allocation: {}. Endpoint field: {}. Worker field: {}.",
                    p.handoff.status,
                    p.handoff
                        .queue_allocation
                        .as_deref()
                        .unwrap_or("unresolved"),
                    p.handoff.endpoint_field.as_deref().unwrap_or("unresolved"),
                    p.handoff.worker_field.as_deref().unwrap_or("unresolved")
                ))
            );
            if let Some(c) = &p.wiring {
                out += "<h2>Selected wiring declaration</h2>\n";
                out += &callable(c, ext);
            }
        }
        "endpoint" => out += &callable(&p.endpoint, ext),
        "worker" => out += &callable(&p.worker, ext),
        "fields-state" => {
            out += &state(&p.endpoint, ext);
            out += &state(&p.worker, ext);
            if let Some(c) = &p.wiring {
                out += &state(c, ext);
            }
        }
        "diagnostic" => {
            if let Some(q) = &p.selection.question {
                out += &paragraph(&format!("Question: {q}"));
            }
            out += &paragraph(
                "Inspect rows describe source-derived possible reasons; they do not diagnose an observed incident.",
            );
            out += &paragraph(
                "Rows concern direct calls in the selected worker body. A later result check does not prevent an earlier call. If a call throws, its normal result is unavailable; these rows do not establish which field assignments completed or any runtime outcome.",
            );
            out += &format!(
                "<p>Inspect {} for source order and call boundaries, {} for retained local changes, and {} for exact evidence.</p>\n",
                link(&format!("{}-worker.{ext}", p.id), "Worker path"),
                link(&format!("{}-fields-state.{ext}", p.id), "Fields and state"),
                link(&format!("sources.{ext}"), "Retained sources"),
            );
            if p.diagnostics.is_empty() {
                out += &paragraph(
                    "No source-derived guard alternative is available for this selected worker. This does not establish successful delivery or absence of failure; missing call or control evidence remains a boundary. Use the inspection links above.",
                );
            } else {
                out += "<table tabIndex=\"0\"><caption>Scroll horizontally: source condition diagnostic matrix</caption><thead><tr><th scope=\"col\">Condition</th><th scope=\"col\">Possible reason</th><th scope=\"col\">Inspect</th><th scope=\"col\">Selected call</th><th scope=\"col\">Source</th></tr></thead><tbody>\n";
                for d in &p.diagnostics {
                    out += &format!(
                        "<tr><td><pre>{}</pre></td><td>{}</td><td>{}</td><td><pre>{}</pre></td><td>{}</td></tr>\n",
                        escape(&d.condition),
                        escape(&d.possible_reason),
                        escape(&d.inspect.join("; ")),
                        escape(&d.selected_call),
                        d.citation_ids
                            .iter()
                            .map(|id| cite(id, ext))
                            .collect::<Vec<_>>()
                            .join(" · ")
                    );
                }
                out += "</tbody></table>\n";
            }
        }
        _ => unreachable!(),
    }
    if p.selection.expand_source_calls
        && let Some(graph) = graph
    {
        if matches!(view, "overview" | "worker") {
            out += &process_calls(p, graph, ext);
        } else {
            out += &format!(
                "<p>{}</p>\n",
                link(
                    &format!("{}-overview.{ext}#source-call-navigation", p.id),
                    "Linked process source context and examined-source identity"
                )
            );
        }
    }
    if !p.human_instructions.is_empty() {
        if view == "overview" {
            out += &operational_instructions(&p.human_instructions);
        } else {
            out += &format!(
                "<p>{}</p>\n",
                link(
                    &format!("{}-overview.{ext}#operational-instructions", p.id),
                    "Operational instructions and captured note identities"
                )
            );
        }
    }
    if !p.authored_paragraphs.is_empty() {
        if view == "overview" {
            out += &authored_paragraphs(&p.authored_paragraphs, ext);
        } else {
            out += &format!(
                "<p>{}</p>\n",
                link(
                    &format!("{}-overview.{ext}#authored-paragraphs", p.id),
                    "Maintained user documentation and original source context"
                )
            );
        }
    }
    out += &format!(
        "<details><summary>Sources and version</summary>{}</details>\n",
        paragraph(&format!(
            "Service: {}. Revision: {}. Snapshot: {}. Service digest: {}. Endpoint symbol: {}. Worker symbol: {}.",
            p.selection.service,
            p.service_revision,
            snapshot,
            p.service_digest,
            p.endpoint.symbol,
            p.worker.symbol
        ))
    );
    out += &format!(
        "<details><summary>Retained limitations</summary><ul>{}</ul></details><p>{}</p></main>\n",
        gaps(&p.limitations, ext),
        link(
            "projection.json",
            "Complete typed projection and retained evidence"
        )
    );
    out
}
fn appendix(p: &BundleProjection, ext: &str) -> String {
    let has_declaration_only = p.pages.iter().any(|page| {
        page.projection_kind
            .is_some_and(ProjectionKind::is_declaration_view)
    });
    let all_declaration_only = p.pages.iter().all(|page| {
        page.projection_kind
            .is_some_and(ProjectionKind::is_declaration_view)
    });
    let has_compiler_control_flow = p
        .pages
        .iter()
        .any(|page| page.projection_kind == Some(ProjectionKind::CompilerControlFlow));
    let index_label = if !has_declaration_only {
        "All processes"
    } else if all_declaration_only && has_compiler_control_flow {
        "All selected declarations and compiler control flow"
    } else if all_declaration_only {
        "All selected declarations"
    } else {
        "All selected pages"
    };
    let mut out = format!(
        "<main><h1>Retained source appendix</h1><p>{}</p>\n",
        link(&format!("index.{ext}"), index_label)
    );
    let mut citations = BTreeMap::new();
    for page in &p.pages {
        for (id, c) in &page.citations {
            citations.entry(id).or_insert((&page.sources, c));
        }
    }
    if let Some(graph) = &p.source_call_graph {
        for node in graph.nodes.values() {
            for (id, c) in &node.citations {
                citations.entry(id).or_insert((&node.sources, c));
            }
        }
    }
    for (id, (source_records, c)) in citations {
        out += &format!(
            "<section id=\"{}\"><h2>{}:{}–{}</h2>\n",
            anchor(id),
            escape(&c.file),
            c.start_line,
            c.end_line
        );
        out += &paragraph(&format!(
            "Citation: {}. Service: {}. Revision: {}. Source ID: {}. Authority: {}. Text digest: {}. Evidence digest: {}. Retained byte range: {}–{}.",
            c.id,
            c.service,
            c.revision,
            c.source_id,
            c.authority,
            c.text_digest,
            c.evidence_digest,
            c.start_byte,
            c.end_byte
        ));
        if let Some(url) = &c.url {
            if url.starts_with("https://") || url.starts_with("http://") {
                out += &format!("<p>{}</p>\n", link(url, "Original source anchor"));
            } else {
                out += &paragraph(&format!("Original source anchor: {url}"));
            }
        } else {
            out += &paragraph(
                "Original source URL unavailable; retained source anchor is local to this appendix.",
            );
        }
        if let Some(source) = source_records.get(&c.source_id) {
            // UTF-8 bounds are checked by the projector; use a checked slice here too.
            if let Some(text) = source.text.get(c.start_byte..c.end_byte) {
                out += &format!("<pre>{}</pre>\n", escape(text));
            }
        }
        out += "</section>\n";
    }
    // Preserve complete source snippets as well as individual citation ranges.
    let mut sources = BTreeMap::new();
    for page in &p.pages {
        for source in page.sources.values() {
            sources
                .entry((&source.service, &source.id))
                .or_insert(source);
        }
    }
    if let Some(graph) = &p.source_call_graph {
        for node in graph.nodes.values() {
            for source in node.sources.values() {
                sources
                    .entry((&source.service, &source.id))
                    .or_insert(source);
            }
        }
    }
    for source in sources.values() {
        out += &format!(
            "<details id=\"{}\"><summary>{}:{}–{}</summary><p>{}</p><pre>{}</pre></details>\n",
            anchor(&format!("source:{}:{}", source.service, source.id)),
            escape(&source.file),
            source.start_line,
            source.end_line,
            escape(&format!(
                "Revision: {}. Authority: {}. Text digest: {}. Evidence digest: {}.",
                source.revision, source.authority, source.text_digest, source.evidence_digest
            )),
            escape(&source.text)
        );
    }
    let mut shown = BTreeSet::new();
    for page in &p.pages {
        for paragraph in &page.authored_paragraphs {
            for (id, source) in &paragraph.source_records {
                let local_anchor = authored_source_anchor(paragraph, id);
                if !shown.insert(local_anchor.clone()) {
                    continue;
                }
                out += &format!(
                    "<section id=\"{}\"><h2>{} authored paragraph context: {}:{}–{}</h2>\n",
                    local_anchor,
                    if paragraph
                        .paragraph
                        .authorship
                        .as_ref()
                        .is_some_and(|a| a.context_migration.is_some())
                    {
                        "Explicitly selected"
                    } else {
                        "Original"
                    },
                    escape(&source.file),
                    source.start_line,
                    source.end_line
                );
                out += &self::paragraph(&format!(
                    "Logical SOURCE ID: {}. Bundle: {}. Operation: {}. Fragment: {}. Revision: {}. Source authority: {}. Text digest: {}. Evidence digest: {}. This source context does not verify the authored prose.",
                    id,
                    paragraph.selection.bundle,
                    paragraph.selection.operation,
                    paragraph.selection.fragment,
                    source.revision,
                    source.authority,
                    source.text_digest,
                    source.evidence_digest
                ));
                out += &format!("<pre>{}</pre></section>\n", escape(&source.text));
            }
        }
    }
    out += "</main>\n";
    out
}
fn html(title: &str, body: &str) -> String {
    format!(
        "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>{}</title><link rel=\"stylesheet\" href=\"style.css\"></head><body>\n{body}</body></html>\n",
        escape(title)
    )
}
const CSS: &str = "html{color:#172d49;background:#f7f4ec;font:16px/1.6 system-ui,sans-serif}main{max-width:1080px;margin:auto;padding:32px 24px;overflow-wrap:anywhere}a{color:#245d97}nav{margin:20px 0}pre{white-space:pre-wrap;overflow-wrap:anywhere;background:#edf0f2;padding:12px}table{border-collapse:collapse;width:100%;display:block;overflow-x:auto}table:focus-visible{outline:2px solid #245d97;outline-offset:3px}caption{text-align:left;font-weight:600}th,td{min-width:160px;padding:8px;border:1px solid #c7d0da;text-align:left;vertical-align:top}th:nth-child(3),td:nth-child(3){min-width:180px}th:nth-child(4),td:nth-child(4){min-width:220px}section,details{border-top:1px solid #c7d0da;padding:12px 0}h1{line-height:1.2}[hidden]{display:none!important}.catalog-controls{display:flex;flex-wrap:wrap;gap:12px}.catalog-controls label{display:flex;flex-direction:column;max-width:100%}.catalog-controls input,.catalog-controls select{font:inherit;box-sizing:border-box;max-width:100%}#catalog-results{padding-left:24px}.catalog-meta,.catalog-detail{display:block}#catalog-results .catalog-identity{border:0;padding:4px 0;font-size:13px}#catalog-results .catalog-identity summary{cursor:pointer;width:fit-content}#catalog-results .catalog-identity small{display:block}#catalog-results .catalog-identity code{overflow-wrap:anywhere}.catalog-pager{display:flex;gap:12px}.catalog-pager button{font:inherit}a:focus-visible,input:focus-visible,select:focus-visible,button:focus-visible,summary:focus-visible{outline:2px solid #245d97;outline-offset:3px}@media(max-width:600px){main{padding:20px 16px}h1{font-size:28px}}\n";

fn validate_projection_kind(projection: &BundleProjection) -> Result<(), ClewError> {
    let graph_counts = projection
        .pages
        .iter()
        .map(super::source::control_flow::validate_page)
        .collect::<Result<Vec<_>, _>>()?;
    let graph_count = graph_counts.iter().sum::<usize>();
    if super::project::uses_source_invocations(projection)
        != (projection.schema == SOURCE_INVOCATION_SCHEMA)
    {
        return Err(invalid(
            "Compiler source-invocation evidence requires its native projection schema",
        ));
    }
    match projection.schema.as_str() {
        SCHEMA => {
            if projection
                .pages
                .iter()
                .any(|page| page.projection_kind.is_some())
            {
                return Err(invalid(
                    "legacy native page projection schema cannot contain projectionKind",
                ));
            }
            if graph_count != 0 {
                return Err(invalid(
                    "legacy native page projection schema cannot contain compiler control-flow panels",
                ));
            }
        }
        DECLARATION_SCHEMA => {
            if projection
                .pages
                .iter()
                .any(|page| page.projection_kind.is_none())
            {
                return Err(invalid(
                    "declaration-capable native page projection requires projectionKind on every page",
                ));
            }
            if !projection
                .pages
                .iter()
                .any(|page| page.projection_kind == Some(ProjectionKind::DeclarationOnly))
            {
                return Err(invalid(
                    "declaration-capable native page projection requires a declaration-only page",
                ));
            }
            if graph_count != 0
                || projection
                    .pages
                    .iter()
                    .any(|page| page.projection_kind == Some(ProjectionKind::CompilerControlFlow))
            {
                return Err(invalid(
                    "declaration projection schema cannot contain compiler control-flow panels",
                ));
            }
        }
        CONTROL_FLOW_SCHEMA => {
            if projection
                .pages
                .iter()
                .any(|page| page.projection_kind.is_none())
            {
                return Err(invalid(
                    "compiler control-flow schema requires projectionKind on every page",
                ));
            }
            if graph_count == 0
                || !projection
                    .pages
                    .iter()
                    .any(|page| page.projection_kind == Some(ProjectionKind::CompilerControlFlow))
            {
                return Err(invalid(
                    "compiler control-flow schema requires at least one graph-bearing page",
                ));
            }
        }
        SOURCE_INVOCATION_SCHEMA => {
            if projection
                .pages
                .iter()
                .any(|page| page.projection_kind.is_none())
            {
                return Err(invalid(
                    "Source-invocation schema requires projectionKind on every page",
                ));
            }
        }
        _ => return Err(invalid("unsupported native page projection schema")),
    }
    for (page, page_graph_count) in projection.pages.iter().zip(graph_counts) {
        let declares_graph = page.projection_kind == Some(ProjectionKind::CompilerControlFlow);
        if declares_graph != (page_graph_count != 0) {
            return Err(invalid(
                "page control-flow kind differs from its retained graph payloads",
            ));
        }
        if page
            .projection_kind
            .is_some_and(ProjectionKind::forbids_expansion)
            && page.selection.expand_data_state
        {
            return Err(invalid(
                "declaration and compiler control-flow pages cannot request data-state expansion",
            ));
        }
        if page.selection.expand_source_calls
            && page
                .projection_kind
                .is_some_and(ProjectionKind::forbids_expansion)
        {
            let graph = projection.source_call_graph.as_ref();
            let has_selected_kotlin_node = graph.is_some_and(|graph| {
                [
                    Some(page.selection.endpoint_declaration.as_str()),
                    Some(page.selection.worker_declaration.as_str()),
                    page.selection.wiring_declaration.as_deref(),
                ]
                .into_iter()
                .flatten()
                .any(|declaration| {
                    graph.nodes.values().any(|node| {
                        node.service == page.selection.service
                            && node.callable.declaration_id == declaration
                            && node.node_projection_kind.is_some()
                    })
                })
            });
            if !has_selected_kotlin_node {
                return Err(invalid(
                    "declaration page source-call expansion requires an admitted compiler exact-call graph root",
                ));
            }
        }
        if page.selection.expand_source_calls && projection.source_call_graph.is_none() {
            return Err(invalid(
                "source-call expansion selection has no retained source-call graph",
            ));
        }
    }
    Ok(())
}

pub(super) fn write(
    output: &Path,
    snapshot: &str,
    p: &BundleProjection,
) -> Result<Value, ClewError> {
    // Portable preflight before constructing or creating any output files.
    validate_projection_kind(p)?;
    if let Some(graph) = &p.source_call_graph {
        super::linked::validate_graph(graph, &p.pages)?;
    }
    let mut names = BTreeSet::new();
    for page in &p.pages {
        if !store::valid_id(&page.id) || !names.insert(page.id.to_ascii_lowercase()) {
            return Err(invalid(
                "native page IDs must be safe and unique ignoring ASCII case",
            ));
        }
    }
    let catalogue = catalogue::rows(p)?;
    let has_declaration_only = p.pages.iter().any(|page| {
        page.projection_kind
            .is_some_and(ProjectionKind::is_declaration_view)
    });
    let has_compiler_control_flow = p
        .pages
        .iter()
        .any(|page| page.projection_kind == Some(ProjectionKind::CompilerControlFlow));
    let mut files = BTreeMap::new();
    let mut page_rows = Vec::new();
    for ext in ["html", "mdx"] {
        let mut bodies = BTreeMap::new();
        let mut index = "<main><h1>Native source documentation</h1>".to_string();
        if ext == "html" {
            index += &catalogue::enhancement(
                &catalogue,
                has_declaration_only,
                has_compiler_control_flow,
            )?;
        }
        index += "<ul id=\"catalog-processes\">\n";
        for page_content in &p.pages {
            if !store::valid_id(&page_content.id) {
                return Err(invalid("native page ID is invalid"));
            }
            index += &format!(
                "<li>{}</li>\n",
                link(
                    &format!("{}-overview.{ext}", page_content.id),
                    &process_label(page_content)
                )
            );
            for (slug, title) in views_for(page_content) {
                bodies.insert(
                    format!("{}-{slug}", page_content.id),
                    (
                        title.to_string(),
                        page(
                            page_content,
                            slug,
                            title,
                            snapshot,
                            ext,
                            p.source_call_graph.as_ref(),
                        ),
                    ),
                );
            }
        }
        index += &format!(
            "</ul><p>{} · {}</p>",
            link(&format!("sources.{ext}"), "Shared source appendix"),
            link("projection.json", "Typed projection")
        );
        if p.source_call_graph.is_some() {
            index += &format!(
                "<p>{}</p>",
                link(
                    &format!("source-calls.{ext}"),
                    "Examined callable bodies and reverse process links"
                )
            );
        }
        index += &format!(
            "<details><summary>Sources and version</summary>{}</details></main>\n",
            paragraph(&format!(
                "Snapshot: {snapshot}. Input digest: {}. Context digest: {}. Selection digest: {}.",
                p.input_digest, p.context_digest, p.selection_digest
            ))
        );
        bodies.insert(
            "index".into(),
            ("Native source documentation".into(), index),
        );
        bodies.insert(
            "sources".into(),
            ("Retained source appendix".into(), appendix(p, ext)),
        );
        if let Some(graph) = &p.source_call_graph {
            bodies.insert(
                "source-calls".into(),
                (
                    "Retained source-call bodies".into(),
                    expanded_sources(graph, ext),
                ),
            );
        }
        for (slug, (title, body)) in bodies {
            // Scalar and source newlines are already escaped. These remaining
            // newlines only format native JSX and must not start MDX paragraphs.
            let body = body.replace('\n', "");
            if ext == "html" {
                let mut row = json!({"id":slug,"contentDigest":digest(&(p, &slug))?,"html":format!("{slug}.html"),"mdx":format!("{slug}.mdx")});
                for process in &p.pages {
                    if VIEWS
                        .iter()
                        .any(|(view, _)| slug == format!("{}-{view}", process.id))
                        && let Some(examined) = &process.examined_sources
                    {
                        row["examinedSourceDigest"] = json!(examined.examined_source_digest);
                        if let Some(state) = &process.data_state {
                            row["dataStateDigest"] = json!(state.data_state_digest);
                        }
                    }
                }
                page_rows.push(row);
            }
            let bytes = if ext == "html" {
                let mut document = html(&title, &body);
                if slug == "index" {
                    document = document.replace(
                        "</head>",
                        "<script src=\"native-reader.js\" defer></script></head>",
                    );
                }
                document.into_bytes()
            } else {
                body.into_bytes()
            };
            files.insert(format!("{slug}.{ext}"), bytes);
        }
    }
    files.insert("style.css".into(), CSS.as_bytes().to_vec());
    files.insert(
        "native-reader.js".into(),
        crate::documentation::reader::SCRIPT.as_bytes().to_vec(),
    );
    files.insert("catalogue.json".into(), serde_json::to_vec_pretty(&json!({"schema":"codeclew-native-catalogue/1.0", "snapshot":snapshot, "inputDigest":p.input_digest, "contextDigest":p.context_digest, "selectionDigest":p.selection_digest, "authority":"SELECTED_AND_EXAMINED_DOCUMENTATION_CONTEXT_NOT_RUNTIME_IMPACT", "rows":catalogue})).map_err(io_error)?);
    files.insert(
        "projection.json".into(),
        serde_json::to_vec_pretty(p).map_err(io_error)?,
    );
    let mut manifest = json!({"schema":"codeclew-native-pages-static-manifest/1.0", "snapshot":snapshot,
        "inputDigest":p.input_digest, "contextDigest":p.context_digest, "selectionDigest":p.selection_digest,
        "projectionDigest":digest(p)?, "mdxProfile":"MDX 3; inert native JSX; no imports, executable expressions, scripts or network dependencies",
        "pages":page_rows,
        "files":files.iter().map(|(path,bytes)|json!({"path":path,"digest":crate::canonical::hash_bytes(bytes)})).collect::<Vec<_>>()});
    let selected_notes: Vec<_> = p
        .pages
        .iter()
        .flat_map(|page| {
            page.human_instructions.iter().map(move |note| json!({
            "pageId":format!("{}-overview", page.id),"processId":page.id,
            "html":format!("{}-overview.html", page.id),"mdx":format!("{}-overview.mdx", page.id),
            "noteId":note.id,"versionDigest":note.version_digest,
            "contentDigest":note.content_digest,"associationDigest":note.association_digest
        }))
        })
        .collect();
    if !selected_notes.is_empty() {
        manifest["selectedNotes"] = json!(selected_notes);
    }
    let selected_authored: Vec<_> = p.pages.iter().flat_map(|page| {
        page.authored_paragraphs.iter().map(move |row| json!({
            "processId":page.id, "html":format!("{}-overview.html", page.id), "mdx":format!("{}-overview.mdx", page.id),
            "bundle":row.selection.bundle, "operation":row.selection.operation, "fragment":row.selection.fragment,
            "paragraphDigest":row.paragraph_digest, "bindingsDigest":row.bindings_digest,
            "publicationDigest":row.publication_digest, "operationDigest":row.operation_digest,
            "sourceSnapshot":row.paragraph.authorship.as_ref().unwrap().source_snapshot
        }))
    }).collect();
    if !selected_authored.is_empty() {
        manifest["selectedAuthoredParagraphs"] = json!(selected_authored);
    }
    if let Some(graph) = &p.source_call_graph {
        manifest["examinedSourceSchema"] = json!("codeclew-native-examined-source/1.0");
        manifest["examinedSourceAuthority"] = json!(graph.authority);
        manifest["reverseExaminedPages"] = json!(graph.reverse_examined_processes.iter().map(|(node, rows)| {
            (node, rows.iter().map(|row| json!({
                "pageId":format!("{}-overview", row.process_id),"processId":row.process_id,
                "html":format!("{}-overview.html", row.process_id),"mdx":format!("{}-overview.mdx", row.process_id),
                "reasons":row.reasons
            })).collect::<Vec<_>>())
        }).collect::<BTreeMap<_, _>>());
    }
    files.insert(
        "manifest.json".into(),
        serde_json::to_vec_pretty(&manifest).map_err(io_error)?,
    );
    if output.exists() {
        let m = fs::symlink_metadata(output).map_err(io_error)?;
        if !m.is_dir() || m.file_type().is_symlink() {
            return Err(invalid("native pages output must be a regular directory"));
        }
        if fs::read_dir(output).map_err(io_error)?.next().is_some() {
            return Err(invalid(
                "native pages output must be empty; choose a new directory",
            ));
        }
    } else {
        fs::create_dir_all(output).map_err(io_error)?;
    }
    for name in files.keys() {
        store::relative(name)?;
        if fs::symlink_metadata(output.join(name)).is_ok() {
            return Err(invalid("native pages generated filename already exists"));
        }
    }
    for (name, data) in files {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(output.join(name))
            .map_err(io_error)?
            .write_all(&data)
            .map_err(io_error)?;
    }
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mdx_source_text_is_inert_and_common_body_links_resolve_by_format() {
        let input = "@EXT@ {evil()} <script> import x from 'evil' `tick` [x] \\ * _\n";
        let text = escape(input);
        assert!(!text.contains('{') && !text.contains('<') && !text.contains('`'));
        assert!(text.contains("&#123;evil()&#125;"));
        assert!(text.contains("&lt;script&gt;"));
        assert!(text.contains("&#64;EXT&#64;"));
        let p = BundleProjection {
            schema: SCHEMA.into(),
            input_digest: "input".into(),
            context_digest: "context".into(),
            selection_digest: "selection".into(),
            pages: vec![],
            source_call_graph: None,
        };
        let temp = tempfile::tempdir().unwrap();
        let out = temp.path().join("bundle");
        let manifest = write(&out, "snapshot/1", &p).unwrap();
        for row in manifest["files"].as_array().unwrap() {
            let bytes = fs::read(out.join(row["path"].as_str().unwrap())).unwrap();
            assert_eq!(crate::canonical::hash_bytes(&bytes), row["digest"]);
        }
        let mdx = fs::read_to_string(out.join("index.mdx")).unwrap();
        let html = fs::read_to_string(out.join("index.html")).unwrap();
        assert!(mdx.contains("sources.mdx"));
        assert!(html.contains("sources.html"));
        assert!(!mdx.contains("@EXT@"));
        assert!(write(&out, "snapshot/1", &p).is_err());
        assert_eq!(fs::read_to_string(out.join("index.mdx")).unwrap(), mdx);
    }
    #[test]
    fn operational_note_text_and_metadata_are_inert_without_rewriting_crlf() {
        let input = "café☕\r\n<Panel>{probe()}</Panel> `code`\r\n";
        let note = HumanInstruction {
            id: "operations".into(),
            title: "<Title>{x}".into(),
            declared_author: "<Author>{y}".into(),
            classification: "policy".into(),
            period: "2026".into(),
            version_digest: "version".into(),
            content_digest: "content".into(),
            association_digest: "association".into(),
            text: input.into(),
            authority: "HUMAN_OR_IMPORTED_UNVERIFIED".into(),
            source_claim_status: "UNASSESSED".into(),
            association: json!({}),
        };
        let body = operational_instructions(&[note]);
        assert!(body.contains("café☕&#13;&#10;&lt;Panel&gt;&#123;probe()&#125;&lt;/Panel&gt;"));
        assert!(body.contains("Declared author: &lt;Author&gt;&#123;y&#125;"));
        assert!(
            body.contains("Note ID: operations. Version digest: version. Content digest: content.")
        );
        assert!(
            body.contains("HUMAN&#95;OR&#95;IMPORTED&#95;UNVERIFIED")
                && body.contains("UNASSESSED")
        );
        assert!(!body.contains("<Panel>") && !body.contains('{') && !body.contains('`'));
    }
    #[test]
    fn case_colliding_page_ids_refuse_before_touching_output() {
        let callable = CallableProjection {
            declaration_id: "decl".into(),
            symbol: "Example.run".into(),
            authority: "SOURCE".into(),
            citation_id: None,
            control_flow: None,
            source_outline: None,
            retained_call_sites: None,
            steps: vec![],
            state: vec![],
            gaps: vec![],
        };
        let make = |id: &str| PageContent {
            id: id.into(),
            title: id.into(),
            projection_kind: None,
            selection: Selection {
                id: id.into(),
                service: "service".into(),
                endpoint_declaration: "endpoint".into(),
                worker_declaration: "worker".into(),
                wiring_declaration: None,
                question: None,
                note_ids: vec![],
                authored_paragraphs: vec![],
                expand_source_calls: false,
                expand_data_state: false,
            },
            service_revision: "revision".into(),
            service_digest: "digest".into(),
            endpoint: callable.clone(),
            worker: callable.clone(),
            wiring: None,
            handoff: HandoffProjection {
                status: "LOCAL_GAP".into(),
                queue_allocation: None,
                endpoint_field: None,
                worker_field: None,
                citation_ids: vec![],
                gaps: vec![],
                limitation: "Unproved".into(),
            },
            diagnostics: vec![],
            citations: BTreeMap::new(),
            observations: BTreeMap::new(),
            sources: BTreeMap::new(),
            limitations: vec![],
            human_instructions: vec![],
            authored_paragraphs: vec![],
            examined_sources: None,
            data_state: None,
        };
        let p = BundleProjection {
            schema: SCHEMA.into(),
            input_digest: "input".into(),
            context_digest: "context".into(),
            selection_digest: "selection".into(),
            pages: vec![make("alpha-flow"), make("Alpha-flow")],
            source_call_graph: None,
        };
        let temp = tempfile::tempdir().unwrap();
        let absent = temp.path().join("new");
        assert!(write(&absent, "snapshot/1", &p).is_err());
        assert!(!absent.exists());
        let empty = temp.path().join("empty");
        fs::create_dir(&empty).unwrap();
        assert!(write(&empty, "snapshot/1", &p).is_err());
        assert!(fs::read_dir(&empty).unwrap().next().is_none());
        let single = BundleProjection {
            pages: vec![make("single")],
            ..p
        };
        let valid = temp.path().join("valid");
        write(&valid, "snapshot/1", &single).unwrap();
        for slug in ["fields-state", "diagnostic"] {
            let mdx = fs::read_to_string(valid.join(format!("single-{slug}.mdx"))).unwrap();
            if slug == "diagnostic" {
                assert!(!mdx.contains("<table ") && !mdx.contains("<tbody>"));
                assert!(mdx.contains("No source-derived guard alternative is available"));
                assert!(
                    mdx.contains("does not establish successful delivery or absence of failure")
                );
                assert!(mdx.contains("href=\"single-worker.mdx\""));
                assert!(mdx.contains("href=\"single-fields-state.mdx\""));
                assert!(mdx.contains("href=\"sources.mdx\""));
            } else {
                assert!(mdx.contains("<table ") && mdx.contains("<tbody>"));
            }
            assert!(!mdx.contains('\n'));
            let html = fs::read_to_string(valid.join(format!("single-{slug}.html"))).unwrap();
            let body = html
                .split_once("<body>\n")
                .unwrap()
                .1
                .strip_suffix("</body></html>\n")
                .unwrap();
            let normalized = body.replace(".html", ".mdx");
            assert_eq!(mdx, normalized);
        }
        let mut guarded = single.clone();
        guarded.pages[0].diagnostics.push(Diagnostic {
            condition: "(ready) is false".into(),
            possible_reason: "The source branch can leave this call unreached.".into(),
            inspect: vec!["ready".into(), "gateway.deliver(request)".into()],
            selected_call: "gateway.deliver(request)".into(),
            citation_ids: vec![],
        });
        let guarded_output = temp.path().join("guarded");
        write(&guarded_output, "snapshot/1", &guarded).unwrap();
        let mdx = fs::read_to_string(guarded_output.join("single-diagnostic.mdx")).unwrap();
        let html = fs::read_to_string(guarded_output.join("single-diagnostic.html")).unwrap();
        assert!(mdx.contains("<tbody><tr>") && mdx.contains("gateway.deliver(request)"));
        assert!(mdx.contains("A later result check does not prevent an earlier call"));
        assert!(mdx.contains("do not establish which field assignments completed"));
        assert!(!mdx.contains("No source-derived guard alternative is available"));
        let body = html
            .split_once("<body>\n")
            .unwrap()
            .1
            .strip_suffix("</body></html>\n")
            .unwrap();
        assert_eq!(mdx, body.replace(".html", ".mdx"));
    }
}
