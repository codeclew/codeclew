//! Closed public presentation types. No native record is serialized into HTML.
use crate::documentation::{io_error, model::Operation, render::escape};
use crate::error::ClewError;
use std::collections::BTreeMap;

pub(super) struct Document {
    title: String,
    pub sections: Vec<Section>,
    pub original_heading: bool,
}
pub(super) struct Section {
    pub reference: String,
    title: String,
    before: String,
    after: String,
    show_heading: bool,
    prose: Vec<(String, bool)>,
    tables: Vec<Table>,
    diagrams: Vec<Diagram>,
    sequences: Vec<Sequence>,
    limits: Vec<String>,
}
struct Table {
    title: String,
    headers: Vec<String>,
    rows: Vec<Vec<String>>,
}
struct Diagram {
    title: String,
    nodes: Vec<Node>,
    edges: Vec<Edge>,
}
struct Sequence {
    actors: Vec<String>,
    steps: Vec<SequenceStep>,
}
struct SequenceStep {
    kind: String,
    from: Option<usize>,
    to: Option<usize>,
    label: String,
    detail: String,
}
struct Node {
    label: String,
    detail: String,
    column: usize,
    row: usize,
}
struct Edge {
    from: usize,
    to: usize,
    label: String,
}

impl Document {
    pub fn new(title: String) -> Self {
        Self {
            title,
            sections: vec![],
            original_heading: false,
        }
    }
}
impl Section {
    pub fn markdown(title: String, text: String) -> Self {
        Self {
            reference: String::new(),
            title,
            before: String::new(),
            after: String::new(),
            show_heading: false,
            prose: vec![(
                crate::documentation::notes::protected_pages::body(&text),
                false,
            )],
            tables: vec![],
            diagrams: vec![],
            sequences: vec![],
            limits: vec![],
        }
    }
}

/// Preserve unselected reader content without assigning it a selected claim's
/// evidence address. Public intervals are explicit caller-owned Markdown blocks.
pub(super) fn surround(section: &mut Section, before: String, after: String) {
    section.before = crate::documentation::notes::protected_pages::body(&before);
    section.after = after;
}
pub(super) fn surround_after(section: &mut Section, after: String) {
    section.after = after;
}

/// Adapt only substantive fields. Source IDs, evidence maps, hashes, revisions,
/// associations and native schemas have no destination in these public types.
pub(super) fn operation(op: &Operation, reader_conditions: &[String]) -> Section {
    let mut section = Section {
        reference: String::new(),
        title: op.title.clone(),
        before: String::new(),
        after: String::new(),
        show_heading: true,
        prose: vec![(op.summary.text.clone(), false)],
        tables: vec![],
        diagrams: vec![],
        sequences: vec![],
        limits: reader_conditions.to_vec(),
    };
    section
        .prose
        .extend(op.explanation.iter().map(|p| (p.text.clone(), p.detail)));
    section
        .prose
        .extend(op.findings.iter().map(|p| (p.text.clone(), false)));
    let actors = op
        .participants
        .iter()
        .enumerate()
        .map(|(i, p)| (p.id.as_str(), (i, p.label.clone())))
        .collect::<BTreeMap<_, _>>();
    if !op.events.is_empty() {
        let mut sequence = Sequence {
            actors: op.participants.iter().map(|p| p.label.clone()).collect(),
            steps: vec![],
        };
        let mut rows = vec![];
        for event in &op.events {
            let actor = |id: &Option<String>| id.as_deref().and_then(|id| actors.get(id));
            let from = actor(&event.from);
            let to = actor(&event.to);
            let label = event
                .reader
                .as_ref()
                .and_then(|r| r.label.as_ref())
                .map(|r| r.text.clone())
                .unwrap_or_else(|| event.text.clone());
            let execution = event
                .reader
                .as_ref()
                .and_then(|r| r.execution.as_ref())
                .map(|r| format!("{}: {}", r.mode, r.meaning.text))
                .unwrap_or_default();
            let detail = format!("{}\n{}\n{}", event.kind, event.text, execution);
            sequence.steps.push(SequenceStep {
                kind: event.kind.clone(),
                from: from.map(|(i, _)| *i),
                to: to.map(|(i, _)| *i),
                label: label.clone(),
                detail,
            });
            rows.push(vec![
                event.kind.clone(),
                from.map(|(_, s)| s.clone()).unwrap_or_default(),
                to.map(|(_, s)| s.clone()).unwrap_or_default(),
                label,
                event.text.clone(),
                execution,
            ]);
        }
        section.sequences.push(sequence);
        section.tables.push(Table {
            title: "Conditions, interactions and outcomes".into(),
            headers: vec![
                "Kind".into(),
                "Sender".into(),
                "Receiver".into(),
                "Step".into(),
                "Details".into(),
                "Execution".into(),
            ],
            rows,
        });
    }
    if let Some(overview) = &op.overview_diagram {
        let mut diagram = Diagram {
            title: "Process overview".into(),
            nodes: vec![],
            edges: vec![],
        };
        let indexes = overview
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.id.as_str(), i))
            .collect::<BTreeMap<_, _>>();
        for node in &overview.nodes {
            let actor = actors
                .get(node.participant.as_str())
                .map(|(_, label)| label.as_str())
                .unwrap_or("");
            diagram.nodes.push(Node {
                label: node.text.clone(),
                detail: format!("{}\n{}", actor, node.text),
                column: node.column.into(),
                row: node.row.into(),
            });
        }
        for edge in &overview.edges {
            if let (Some(&from), Some(&to)) = (
                indexes.get(edge.from.as_str()),
                indexes.get(edge.to.as_str()),
            ) {
                diagram.edges.push(Edge {
                    from,
                    to,
                    label: edge.text.clone(),
                });
            }
        }
        section.diagrams.push(diagram);
    }
    for contract in &op.interface_contracts {
        section.tables.push(Table {
            title: format!("{} · {}", contract.title, contract.kind),
            headers: vec!["Field".into(), "Contract".into()],
            rows: contract
                .rows
                .iter()
                .map(|r| vec![r.label.clone(), r.value.clone()])
                .collect(),
        });
    }
    for visual in &op.visuals {
        section.prose.push((
            format!(
                "{}\n\n{}\n\n{}",
                visual.title, visual.purpose.text, visual.scope.text
            ),
            false,
        ));
        let indexes = visual
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.id.as_str(), i))
            .collect::<BTreeMap<_, _>>();
        if !visual.nodes.is_empty() {
            section.diagrams.push(Diagram {
                title: visual.title.clone(),
                nodes: visual
                    .nodes
                    .iter()
                    .enumerate()
                    .map(|(i, n)| Node {
                        label: n.meaning.text.clone(),
                        detail: n.meaning.text.clone(),
                        column: i % 3,
                        row: i / 3,
                    })
                    .collect(),
                edges: visual
                    .edges
                    .iter()
                    .filter_map(|e| {
                        Some(Edge {
                            from: *indexes.get(e.from.as_str())?,
                            to: *indexes.get(e.to.as_str())?,
                            label: e.meaning.text.clone(),
                        })
                    })
                    .collect(),
            });
        }
        if !visual.rules.is_empty() {
            section.tables.push(Table {
                title: format!(
                    "{} · {}",
                    visual.title,
                    visual.hit_policy.as_deref().unwrap_or("Decision rules")
                ),
                headers: vec!["Condition".into(), "Outcome".into()],
                rows: visual
                    .rules
                    .iter()
                    .map(|r| vec![r.condition.text.clone(), r.outcome.text.clone()])
                    .collect(),
            });
        }
        if let Some(policy) = &visual.policy_explanation {
            section.prose.push((policy.text.clone(), false));
        }
        if let Some(after) = &visual.after_selection {
            section.prose.push((after.text.clone(), false));
        }
    }
    if let Some(flow) = &op.dataflow {
        let indexes = flow
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.id.as_str(), i))
            .collect::<BTreeMap<_, _>>();
        section.diagrams.push(Diagram {
            title: "Data flow".into(),
            nodes: flow
                .nodes
                .iter()
                .enumerate()
                .map(|(i, n)| Node {
                    label: n.meaning.text.clone(),
                    detail: format!(
                        "{} · {} · {}\n{}",
                        n.service, n.kind, n.representation, n.meaning.text
                    ),
                    column: i % 3,
                    row: i / 3,
                })
                .collect(),
            edges: flow
                .edges
                .iter()
                .filter_map(|e| {
                    Some(Edge {
                        from: *indexes.get(e.from.as_str())?,
                        to: *indexes.get(e.to.as_str())?,
                        label: format!("{} · {} · {}", e.kind, e.meaning.text, e.uncertainty),
                    })
                })
                .collect(),
        });
        section.tables.push(Table {
            title: "Data transformations and uncertainty".into(),
            headers: vec!["Kind".into(), "Meaning".into(), "Uncertainty".into()],
            rows: flow
                .edges
                .iter()
                .map(|e| {
                    vec![
                        e.kind.clone(),
                        e.meaning.text.clone(),
                        e.uncertainty.clone(),
                    ]
                })
                .collect(),
        });
    }
    section
}

fn prose(text: &str) -> String {
    let mut html = String::new();
    let mut block = String::new();
    let mut fence: Option<(char, usize)> = None;
    for line in text.lines() {
        let trimmed = line.trim_start();
        let first = trimmed.chars().next().unwrap_or(' ');
        let count = trimmed.chars().take_while(|c| *c == first).count();
        let fenced = fence.is_some();
        if matches!(first, '`' | '~') && count >= 3 {
            if let Some((marker, length)) = fence {
                if marker == first && count >= length && trimmed[count..].trim().is_empty() {
                    fence = None;
                }
            } else {
                fence = Some((first, count));
            }
        }
        let heading = !fenced
            && fence.is_none()
            && first == '#'
            && (1..=6).contains(&count)
            && line.len() - trimmed.len() <= 3
            && trimmed[count..].starts_with(' ');
        if heading {
            if !block.is_empty() {
                html += &format!("<div data-clew-markdown>{}</div>", escape(&block));
                block.clear();
            }
            html += &format!(
                "<h{count}><span data-clew-inline>{}</span></h{count}>",
                escape(trimmed[count..].trim())
            );
        } else {
            block.push_str(line);
            block.push('\n');
        }
    }
    if !block.is_empty() {
        html += &format!("<div data-clew-markdown>{}</div>", escape(&block));
    }
    html
}

// Keep diagram labels on word boundaries; full labels remain in title/details.
fn label_lines(text: &str, width: usize, limit: usize) -> Vec<String> {
    let mut rows = vec![String::new()];
    let words: Vec<_> = text.split_whitespace().collect();
    for (index, word) in words.iter().enumerate() {
        let row = rows.last_mut().unwrap();
        if !row.is_empty() && row.chars().count() + 1 + word.chars().count() > width {
            if rows.len() == limit {
                rows.last_mut().unwrap().push('…');
                break;
            }
            rows.push(String::new());
        }
        let row = rows.last_mut().unwrap();
        if !row.is_empty() {
            row.push(' ');
        }
        if word.chars().count() > width {
            row.extend(word.chars().take(width.saturating_sub(1)));
            row.push('…');
        } else {
            row.push_str(word);
        }
        if index + 1 == words.len() {
            break;
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_keeps_headings_and_authored_boundaries_without_generic_disclaimers() {
        let mut document = Document::new("Manual".into());
        document.sections.push(Section::markdown("Refresh".into(),
            "# Weather\n\n## Refresh `cityId`\n\nHTTP 304 returns before outbox creation.\n\n```text\n# Literal code\n```".into()));
        let html = render(&document).unwrap();
        assert!(html.contains("<h1><span data-clew-inline>Weather</span></h1>"));
        assert!(html.contains("<h2><span data-clew-inline>Refresh `cityId`</span></h2>"));
        assert!(html.contains("HTTP 304 returns before outbox creation."));
        assert!(html.contains("```text\n# Literal code\n```"));
        assert!(!html.contains("<h1><span data-clew-inline>Literal code"));
        assert!(!html.contains("Human/imported prose is unassessed"));
        assert!(!html.contains("current behavior is not reverified by this copy"));
        assert!(!html.contains("What remains uncertain"));
    }

    #[test]
    fn graph_labels_wrap_at_words_and_signal_truncation() {
        assert_eq!(
            label_lines("Otherwise read fixture or fetch live data", 30, 3),
            vec!["Otherwise read fixture or", "fetch live data"]
        );
        assert_eq!(
            label_lines("one two three four five six", 8, 2),
            vec!["one two", "three…"]
        );
    }
}

fn sequence(sequence: &Sequence, section: usize, index: usize) -> String {
    let prefix = format!("sequence-{section}-{index}");
    let width = sequence.actors.len().max(2) * 240;
    let height = sequence.steps.len() * 100 + 100;
    let x = |actor: usize| actor * 240 + 120;
    let mut html = format!(
        "<figure><figcaption>Process sequence</figcaption><button type=\"button\" data-expand=\"{prefix}\">Expand diagram</button><div class=\"diagram\" id=\"{prefix}\"><button type=\"button\" class=\"diagram-close\" data-close-diagram>Close expanded diagram</button><svg role=\"group\" aria-label=\"Process sequence\" viewBox=\"0 0 {width} {height}\"><defs><marker id=\"{prefix}-arrow\" markerWidth=\"8\" markerHeight=\"8\" refX=\"7\" refY=\"4\" orient=\"auto\"><path d=\"M0 0 L8 4 L0 8\" fill=\"#527464\"/></marker></defs>"
    );
    for (i, actor) in sequence.actors.iter().enumerate() {
        let px = x(i);
        html += &format!(
            "<text x=\"{px}\" y=\"30\" text-anchor=\"middle\" font-size=\"16\">{}</text><path d=\"M{px} 50 L{px} {height}\" stroke=\"#aabbad\" stroke-dasharray=\"5 4\"/>",
            escape(actor)
        );
    }
    for (i, step) in sequence.steps.iter().enumerate() {
        let y = i * 100 + 110;
        html += &format!(
            "<a href=\"#{prefix}-detail-{i}\" data-open-detail=\"{prefix}-detail-{i}\"><title>{}: {}</title>",
            escape(&step.kind),
            escape(&step.label)
        );
        if let (Some(from), Some(to)) = (step.from, step.to) {
            let left = x(from);
            let right = x(to);
            let dash = if step.kind == "return" || step.kind == "declared-boundary-return" {
                "stroke-dasharray=\"5 4\""
            } else {
                ""
            };
            let path = if from == to {
                format!("M{left} {y} h80 v30 h-80")
            } else {
                format!("M{left} {y} L{right} {y}")
            };
            html += &format!(
                "<path d=\"{path}\" fill=\"none\" stroke=\"#527464\" stroke-width=\"2\" {dash} marker-end=\"url(#{prefix}-arrow)\"/>"
            );
        } else {
            html += &format!(
                "<rect x=\"10\" y=\"{}\" width=\"{}\" height=\"70\" rx=\"6\" fill=\"#f0f5ee\" stroke=\"#aabbad\"/>",
                y - 48,
                width - 20
            );
        }
        let label = format!("{}: {}", step.kind, step.label);
        for (line, text) in label_lines(&label, 60, 3).iter().enumerate() {
            html += &format!(
                "<text x=\"{}\" y=\"{}\" text-anchor=\"middle\" font-size=\"13\">{}</text>",
                width / 2,
                y - 22 + line * 17,
                escape(text)
            );
        }
        html += "</a>";
    }
    html += "</svg></div>";
    for (i, step) in sequence.steps.iter().enumerate() {
        html += &format!(
            "<details id=\"{prefix}-detail-{i}\"><summary>{}: {}</summary>{}</details>",
            escape(&step.kind),
            escape(&step.label),
            prose(&step.detail)
        );
    }
    html += "</figure>";
    html
}

fn diagram(diagram: &Diagram, section: usize, index: usize) -> String {
    let prefix = format!("diagram-{section}-{index}");
    let width = (diagram.nodes.iter().map(|n| n.column).max().unwrap_or(0) + 1) * 300;
    let height = (diagram.nodes.iter().map(|n| n.row).max().unwrap_or(0) + 1) * 120;
    let position = |n: &Node| (n.column * 300 + 150, n.row * 120 + 60);
    let mut html = format!(
        "<figure><figcaption>{}</figcaption><button type=\"button\" data-expand=\"{prefix}\">Expand diagram</button><div class=\"diagram\" id=\"{prefix}\"><button type=\"button\" class=\"diagram-close\" data-close-diagram>Close expanded diagram</button><svg role=\"group\" aria-label=\"{}\" viewBox=\"0 0 {width} {height}\"><defs><marker id=\"{prefix}-arrow\" markerWidth=\"8\" markerHeight=\"8\" refX=\"7\" refY=\"4\" orient=\"auto\"><path d=\"M0 0 L8 4 L0 8\" fill=\"#527464\"/></marker></defs>",
        escape(&diagram.title),
        escape(&diagram.title)
    );
    for (i, edge) in diagram.edges.iter().enumerate() {
        let (a, b) = position(&diagram.nodes[edge.from]);
        let (c, d) = position(&diagram.nodes[edge.to]);
        let (mut x1, mut y1, mut x2, mut y2) = (a as isize, b as isize, c as isize, d as isize);
        let horizontal = (x2 - x1).abs() >= (y2 - y1).abs();
        let path = if edge.from == edge.to {
            format!("M{} {} h30 v54 h-30", x1 + 130, y1 - 16)
        } else {
            if horizontal {
                let direction = (x2 - x1).signum();
                x1 += direction * 130;
                x2 -= direction * 130;
            } else {
                let direction = (y2 - y1).signum();
                y1 += direction * 38;
                y2 -= direction * 38;
            }
            format!("M{x1} {y1} L{x2} {y2}")
        };
        let label = edge.label.chars().take(60).collect::<String>();
        let (tx, ty) = if horizontal {
            ((x1 + x2) / 2, (y1 + y2) / 2 - 48)
        } else {
            ((x1 + x2) / 2 + 12, (y1 + y2) / 2)
        };
        html += &format!(
            "<a href=\"#{prefix}-edge-{i}\" data-open-detail=\"{prefix}-edge-{i}\"><title>{}</title><path d=\"{path}\" stroke=\"#527464\" fill=\"none\" stroke-width=\"2\" marker-end=\"url(#{prefix}-arrow)\"/><text x=\"{tx}\" y=\"{ty}\" text-anchor=\"middle\" font-size=\"12\">{}</text></a>",
            escape(&edge.label),
            escape(&label)
        );
    }
    for (i, node) in diagram.nodes.iter().enumerate() {
        let (x, y) = position(node);
        html += &format!(
            "<a href=\"#{prefix}-detail-{i}\" data-open-detail=\"{prefix}-detail-{i}\"><title>{}</title><rect x=\"{}\" y=\"{}\" width=\"260\" height=\"76\" rx=\"10\" fill=\"#f0f6f1\" stroke=\"#779484\"/>",
            escape(&node.label),
            x - 130,
            y - 38
        );
        for (line, text) in label_lines(&node.label, 30, 3).iter().enumerate() {
            html += &format!(
                "<text x=\"{x}\" y=\"{}\" text-anchor=\"middle\" font-size=\"13\">{}</text>",
                y - 17 + line * 17,
                escape(text)
            );
        }
        html += "</a>";
    }
    html += "</svg></div>";
    for (i, node) in diagram.nodes.iter().enumerate() {
        html += &format!(
            "<details id=\"{prefix}-detail-{i}\"><summary>{}</summary>{}</details>",
            escape(&node.label),
            prose(&node.detail)
        );
    }
    for (i, edge) in diagram.edges.iter().enumerate() {
        html += &format!(
            "<details id=\"{prefix}-edge-{i}\"><summary>Transition: {}</summary>{}</details>",
            escape(&edge.label),
            prose(&format!(
                "{} → {}\n\n{}",
                diagram.nodes[edge.from].label, diagram.nodes[edge.to].label, edge.label
            ))
        );
    }
    html += "</figure>";
    html
}

pub(super) fn render(document: &Document) -> Result<String, ClewError> {
    let heading = if document.original_heading {
        format!("<p class=\"page-title\">{}</p>", escape(&document.title))
    } else {
        format!("<h1>{}</h1>", escape(&document.title))
    };
    let mut body = format!("<header>{heading}<nav aria-label=\"Page sections\">");
    for (i, s) in document.sections.iter().enumerate() {
        body += &format!("<a href=\"#section-{i}\">{}</a> ", escape(&s.title));
    }
    body += "</nav></header><main>";
    for (i, s) in document.sections.iter().enumerate() {
        if !s.before.is_empty() {
            body += &prose(&s.before);
        }
        body += &format!(
            "<section id=\"section-{i}\" data-clew-reader=\"{}\">",
            escape(&s.reference)
        );
        if s.show_heading {
            body += &format!("<h2>{}</h2>", escape(&s.title));
        }
        for (text, detail) in &s.prose {
            if *detail {
                body += &format!(
                    "<details><summary>Technical details</summary>{}</details>",
                    prose(text)
                );
            } else {
                body += &prose(text);
            }
        }
        for (j, seq) in s.sequences.iter().enumerate() {
            body += &sequence(seq, i, j);
        }
        for (j, d) in s.diagrams.iter().enumerate() {
            body += &diagram(d, i, j);
        }
        for table in &s.tables {
            body += &format!(
                "<h3>{}</h3><div class=\"table-wrap\"><table><thead><tr>",
                escape(&table.title)
            );
            for header in &table.headers {
                body += &format!("<th>{}</th>", escape(header));
            }
            body += "</tr></thead><tbody>";
            for row in &table.rows {
                body += "<tr>";
                for cell in row {
                    body += &format!("<td>{}</td>", prose(cell));
                }
                body += "</tr>";
            }
            body += "</tbody></table></div>";
        }
        if !s.limits.is_empty() {
            body += "<aside><h3>Conditions and limits</h3><ul>";
            for limit in &s.limits {
                body += &format!("<li>{}</li>", escape(limit));
            }
            body += "</ul></aside>";
        }
        body += "</section>";
        if !s.after.is_empty() {
            body += &prose(&s.after);
        }
    }
    body += "</main>";
    let script = include_str!("../../../assets/documentation/markdown.js");
    let style = include_str!("../../../assets/documentation/markdown.css");
    // No machine JSON or external dependency is emitted. All browser data is
    // already represented by the closed public HTML/SVG view above.
    let html = format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>{}</title><style>{style}
body{{margin:24px auto;max-width:1100px;padding:0 20px;font:17px/1.6 system-ui;color:#23352c;background:#fcfdfb}}section{{padding:24px 0;border-top:1px solid #ccd8ce}}a{{color:#245b40}}table{{border-collapse:collapse;width:100%}}td,th{{border:1px solid #ccd8ce;padding:8px;text-align:left;vertical-align:top}}.table-wrap,.diagram{{overflow:auto}}svg{{display:block;width:100%;min-width:540px;max-height:700px}}.diagram.expanded{{position:fixed;inset:20px;z-index:10;background:white;box-shadow:0 0 0 30px #0008;overflow:auto}}.diagram.expanded svg{{max-height:none;min-width:900px}}.diagram-close{{display:none}}.expanded .diagram-close{{display:block;position:sticky;top:0;left:0;z-index:1}}details,aside{{margin:16px 0;padding:12px;background:#f0f5ee}}summary{{cursor:pointer}}button{{padding:8px 14px;cursor:pointer}}nav a{{display:inline-block;margin:4px 12px 4px 0}}:target{{outline:2px solid #779484;scroll-margin:20px}}
</style></head><body>{body}<script>{script}
document.querySelectorAll('[data-clew-inline]').forEach(n=>n.innerHTML=clewMarkdown.inline(n.textContent));
clewMarkdown.enhance(document);
let diagramOpener=null;
function closeDiagrams(restore){{document.querySelectorAll('.diagram.expanded').forEach(d=>{{d.classList.remove('expanded');d.removeAttribute('role');d.removeAttribute('aria-modal');}});document.querySelectorAll('[data-expand]').forEach(b=>b.setAttribute('aria-expanded','false'));if(restore&&diagramOpener)diagramOpener.focus();diagramOpener=null;}}
document.addEventListener('click',function(e){{if(e.target.closest('[data-close-diagram]')){{closeDiagrams(true);return;}}const button=e.target.closest('[data-expand]');if(button){{const figure=document.getElementById(button.dataset.expand),opened=figure.classList.contains('expanded');closeDiagrams(false);if(!opened){{diagramOpener=button;figure.classList.add('expanded');figure.setAttribute('role','dialog');figure.setAttribute('aria-modal','true');button.setAttribute('aria-expanded','true');figure.querySelector('[data-close-diagram]').focus();}}}}const link=e.target.closest('[data-open-detail]');if(link){{e.preventDefault();history.replaceState(null,'',link.getAttribute('href'));closeDiagrams(false);const detail=document.getElementById(link.dataset.openDetail);detail.open=true;detail.scrollIntoView({{block:'center'}});detail.querySelector('summary').focus();}}}});
document.addEventListener('keydown',function(e){{const expanded=document.querySelector('.diagram.expanded');if(!expanded)return;if(e.key==='Escape'){{e.preventDefault();closeDiagrams(true);}}if(e.key==='Tab'){{const controls=Array.from(expanded.querySelectorAll('button,a[href]'));if(!controls.length)return;const first=controls[0],last=controls[controls.length-1];if(e.shiftKey&&document.activeElement===first){{e.preventDefault();last.focus();}}else if(!e.shiftKey&&document.activeElement===last){{e.preventDefault();first.focus();}}}}}});
</script></body></html>"#,
        escape(&document.title)
    );
    // Keep the fallible result contract shared with other native publishers.
    String::from_utf8(html.into_bytes()).map_err(io_error)
}
