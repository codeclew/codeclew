//! Deterministic offline rendering for a structured endpoint operation answer.
//!
//! This validates packet binding, evidence labels, and tree shape. It does not
//! review semantic correctness or publish documentation.

use super::{
    digest, invalid,
    proposals::{Claim, Step},
};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

const ANSWER_SCHEMA: &str = "codeclew-operation-answer/1.0";
const PACKET_SCHEMA: &str = "codeclew-documentation-reader-packet/1.0";
const STEP_KINDS: &[&str] = &["action", "decision", "try", "return", "throw", "loop"];

pub(super) fn output_schema() -> Value {
    serde_json::from_str(include_str!(
        "../../../../schemas/documentation/operation-answer.schema.json"
    ))
    .expect("operation answer schema is valid JSON")
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OperationAnswer {
    schema: String,
    packet_digest: String,
    title: String,
    summary: Claim,
    steps: Vec<Step>,
    uncertainties: Vec<String>,
}

pub(super) struct RenderedAnswer {
    pub(super) html: String,
    pub(super) markdown: String,
    pub(super) answer: Value,
}

/// Validate an answer against the exact compact packet and render all document
/// views from the same authored step tree.
pub(super) fn validate_and_render(
    packet: &Value,
    answer: Value,
) -> Result<RenderedAnswer, crate::error::ClewError> {
    if packet["schema"] != PACKET_SCHEMA || packet["profile"] != "endpoint-context-v3" {
        return Err(invalid(
            "operation answer requires an endpoint reader packet",
        ));
    }
    let citations = packet["citations"]
        .as_object()
        .ok_or_else(|| invalid("reader packet has no citation-label map"))?;
    let known_labels: BTreeSet<_> = citations.keys().cloned().collect();
    if packet_evidence_labels(packet)? != known_labels {
        return Err(invalid(
            "reader packet citation labels do not match its displayed evidence labels",
        ));
    }

    let parsed: OperationAnswer = serde_json::from_value(answer.clone())
        .map_err(|_| invalid("operation answer has an invalid schema or shape"))?;
    if parsed.schema != ANSWER_SCHEMA {
        return Err(invalid("unsupported operation-answer schema"));
    }
    let declared_packet_digest = packet["packetDigest"]
        .as_str()
        .ok_or_else(|| invalid("reader packet has no packetDigest"))?;
    let mut packet_without_digest = packet.clone();
    packet_without_digest
        .as_object_mut()
        .ok_or_else(|| invalid("reader packet must be a JSON object"))?
        .remove("packetDigest");
    let actual_packet_digest = digest(&packet_without_digest)?;
    if declared_packet_digest != actual_packet_digest
        || parsed.packet_digest != actual_packet_digest
    {
        return Err(invalid(
            "operation answer packetDigest does not match the compact reader packet",
        ));
    }
    if parsed.title.trim().is_empty() {
        return Err(invalid("operation answer title must not be empty"));
    }
    validate_claim(&parsed.summary, &known_labels, "summary")?;
    if parsed.steps.is_empty() {
        return Err(invalid("operation answer must contain at least one step"));
    }
    validate_steps(&parsed.steps, &known_labels)?;
    if parsed
        .uncertainties
        .iter()
        .any(|item| item.trim().is_empty())
    {
        return Err(invalid("operation answer uncertainties must not be empty"));
    }

    let evidence_index = evidence_index(citations);
    let html = render_html(packet, &parsed, citations, &evidence_index);
    let markdown = render_markdown(packet, &parsed, citations, &evidence_index);
    Ok(RenderedAnswer {
        html,
        markdown,
        answer,
    })
}

fn packet_evidence_labels(packet: &Value) -> Result<BTreeSet<String>, crate::error::ClewError> {
    fn visit(value: &Value, labels: &mut BTreeSet<String>) -> Result<(), crate::error::ClewError> {
        match value {
            Value::Array(values) => {
                for value in values {
                    visit(value, labels)?;
                }
            }
            Value::Object(values) => {
                for (name, value) in values {
                    if name == "evidence" {
                        let values = value.as_array().ok_or_else(|| {
                            invalid("reader packet evidence fields must be arrays")
                        })?;
                        for label in values {
                            let label = label.as_str().ok_or_else(|| {
                                invalid("reader packet evidence labels must be strings")
                            })?;
                            if label.trim().is_empty() {
                                return Err(invalid(
                                    "reader packet evidence labels must not be empty",
                                ));
                            }
                            labels.insert(label.to_owned());
                        }
                    }
                    visit(value, labels)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    let mut labels = BTreeSet::new();
    visit(packet, &mut labels)?;
    Ok(labels)
}

fn validate_claim(
    claim: &Claim,
    known_labels: &BTreeSet<String>,
    location: &str,
) -> Result<(), crate::error::ClewError> {
    if claim.text.trim().is_empty() {
        return Err(invalid(format!("{location} claim text must not be empty")));
    }
    if claim.evidence.is_empty() {
        return Err(invalid(format!("{location} claim needs packet evidence")));
    }
    for label in &claim.evidence {
        validate_evidence_label(label, known_labels, location)?;
    }
    if claim
        .uncertainty
        .as_ref()
        .is_some_and(|text| text.trim().is_empty())
    {
        return Err(invalid(format!("{location} uncertainty must not be empty")));
    }
    if !claim.checks.is_empty() {
        return Err(invalid(format!(
            "{location} checks are not supported by the operation-answer contract"
        )));
    }
    Ok(())
}

fn validate_evidence_label(
    label: &str,
    known_labels: &BTreeSet<String>,
    location: &str,
) -> Result<(), crate::error::ClewError> {
    if label.trim().is_empty() || !known_labels.contains(label) {
        return Err(invalid(format!(
            "{location} cites an unknown compact-packet evidence label"
        )));
    }
    Ok(())
}

fn validate_steps(
    steps: &[Step],
    known_labels: &BTreeSet<String>,
) -> Result<(), crate::error::ClewError> {
    for (index, step) in steps.iter().enumerate() {
        let location = format!("step {}", index + 1);
        validate_step(step, known_labels, &location)?;
    }
    Ok(())
}

fn validate_step(
    step: &Step,
    known_labels: &BTreeSet<String>,
    location: &str,
) -> Result<(), crate::error::ClewError> {
    if !STEP_KINDS.contains(&step.kind.as_str()) {
        return Err(invalid(format!("{location} has an unsupported kind")));
    }
    validate_claim(&step.meaning, known_labels, location)?;
    for (name, value) in [
        ("from", step.from.as_deref()),
        ("to", step.to.as_deref()),
        ("interaction", step.interaction.as_deref()),
    ] {
        if value.is_some_and(|value| value.trim().is_empty()) {
            return Err(invalid(format!("{location} {name} must not be empty")));
        }
    }

    match step.kind.as_str() {
        "decision" if step.children.is_empty() => {
            return Err(invalid(format!(
                "{location} decision has no supplied true path"
            )));
        }
        "try" | "loop" if step.children.is_empty() => {
            return Err(invalid(format!(
                "{location} requires a supplied child path"
            )));
        }
        "action" if !step.otherwise.is_empty() => {
            return Err(invalid(format!(
                "{location} otherwise path requires a decision, try, or loop"
            )));
        }
        "return" | "throw" if !step.children.is_empty() || !step.otherwise.is_empty() => {
            return Err(invalid(format!(
                "{location} terminal step cannot have child paths"
            )));
        }
        _ => {}
    }

    for (group, children) in [("child", &step.children), ("otherwise", &step.otherwise)] {
        for (index, child) in children.iter().enumerate() {
            validate_step(
                child,
                known_labels,
                &format!("{location} {group} {}", index + 1),
            )?;
        }
    }
    Ok(())
}

fn evidence_index(citations: &serde_json::Map<String, Value>) -> BTreeMap<String, usize> {
    let mut labels: Vec<_> = citations.keys().cloned().collect();
    labels.sort();
    labels
        .into_iter()
        .enumerate()
        .map(|(index, label)| (label, index + 1))
        .collect()
}

fn render_html(
    packet: &Value,
    answer: &OperationAnswer,
    citations: &serde_json::Map<String, Value>,
    evidence_index: &BTreeMap<String, usize>,
) -> String {
    let language = packet["documentationLanguage"].as_str().unwrap_or("en");
    let mut html = String::from("<!doctype html><html lang=\"");
    html.push_str(&html_escape(language));
    html.push_str("\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>");
    html.push_str(&html_escape(&answer.title));
    html.push_str("</title><style>");
    html.push_str(OFFLINE_STYLE);
    html.push_str("</style></head><body><main>");
    html.push_str(&format!(
        "<h1>{}</h1><p class=\"review-status\"><strong>DRAFT / UNREVIEWED</strong> — Structure and evidence-label binding were validated. Semantic correctness was not reviewed.</p><p class=\"packet-digest\">Packet digest: <code>{}</code></p>",
        html_escape(&answer.title),
        html_escape(&answer.packet_digest)
    ));
    html.push_str("<section><h2>Summary</h2>");
    html.push_str(&render_claim_html(&answer.summary, evidence_index));
    html.push_str("</section><section><h2>Ordered operation</h2>");
    html.push_str(&render_html_steps(&answer.steps, evidence_index, true));
    html.push_str("</section><section><h2>Offline operation tree</h2><figure class=\"step-tree\"><figcaption>Derived from the supplied answer steps</figcaption>");
    html.push_str(&render_tree_html(&answer.steps, evidence_index));
    html.push_str("</figure></section>");
    html.push_str(&render_decision_tables_html(&answer.steps, evidence_index));
    html.push_str(&render_packet_fact_tables_html(packet, evidence_index));
    html.push_str(&render_packet_limits_html(packet));
    html.push_str(&render_uncertainties_html(answer));
    html.push_str(&render_evidence_index_html(citations, evidence_index));
    html.push_str("</main></body></html>");
    html
}

fn render_markdown(
    packet: &Value,
    answer: &OperationAnswer,
    citations: &serde_json::Map<String, Value>,
    evidence_index: &BTreeMap<String, usize>,
) -> String {
    let mut markdown = format!(
        "# {}\n\n> **DRAFT / UNREVIEWED.** Structure and evidence-label binding were validated; semantic correctness was not reviewed.\n\nPacket digest: `{}`\n\n## Summary\n\n{}\n\n## Ordered operation\n\n{}\n\n## Offline operation tree\n\n{}\n\n",
        markdown_escape(&answer.title),
        markdown_escape(&answer.packet_digest),
        render_claim_markdown(&answer.summary, evidence_index),
        render_markdown_steps(&answer.steps, evidence_index, 0, true),
        markdown_tree_block(&answer.steps)
    );
    markdown.push_str(&render_decision_tables_markdown(
        &answer.steps,
        evidence_index,
    ));
    markdown.push_str(&render_packet_fact_tables_markdown(packet, evidence_index));
    markdown.push_str(&render_packet_limits_markdown(packet));
    markdown.push_str(&render_uncertainties_markdown(answer));
    markdown.push_str(&render_evidence_index_markdown(citations, evidence_index));
    markdown
}

const OFFLINE_STYLE: &str = r#"
:root{color-scheme:light dark;font:16px/1.55 system-ui,sans-serif;--line:#8792a2;--panel:#171b22;--accent:#73b7ff}
*{box-sizing:border-box}body{margin:0;background:#101319;color:#e8edf5}main{max-width:1120px;margin:auto;padding:2rem}
h1,h2,h3{line-height:1.2}h2{margin-top:2.2rem;border-bottom:1px solid #394252;padding-bottom:.45rem}
a{color:var(--accent)}code{overflow-wrap:anywhere}.review-status{padding:.85rem 1rem;border-left:4px solid #d99e45;background:#29231a}
.packet-digest{color:#bac4d3}.claim,.step-node,.tree-node{border:1px solid #394252;border-radius:.55rem;padding:.8rem 1rem;margin:.55rem 0;background:var(--panel)}
.claim-text,.step-text{white-space:pre-wrap}.claim-uncertainty{color:#ffd08a}.citations{display:inline-flex;gap:.45rem;flex-wrap:wrap;margin-left:.45rem;font-size:.9em}
.citation{border:1px solid #52627a;border-radius:1rem;padding:.05rem .5rem;text-decoration:none}.step-kind{font-size:.75em;text-transform:uppercase;letter-spacing:.06em;color:#9ed0ff;margin-right:.55rem}
.step-meta{color:#b7c1d0;font-size:.9em}.ordered-steps,.nested-steps{padding-left:1.5rem}.path-group{margin:.5rem 0 .75rem 1rem;padding-left:.8rem;border-left:2px solid #52627a}.path-label{font-weight:650;color:#bdc9dc}
.step-tree ul{list-style:none;margin:.25rem 0 .25rem 1rem;padding-left:1rem;border-left:2px solid var(--line)}.step-tree li{position:relative;padding:.25rem 0 .25rem .4rem}.step-tree li::before{content:"";position:absolute;left:-1rem;top:1.25rem;width:.8rem;border-top:2px solid var(--line)}
.tree-node{display:inline-block;max-width:100%}.tree-branch-label{margin:.35rem 0 0 1rem;color:#bdc9dc;font-size:.9em}
table{border-collapse:collapse;width:100%;margin:1rem 0 1.5rem}caption{text-align:left;font-weight:700;margin:.5rem 0}th,td{border:1px solid #596273;padding:.5rem .65rem;text-align:left;vertical-align:top;overflow-wrap:anywhere}th{background:#242b36}
.evidence-index,.limitations,.uncertainties{padding-left:1.4rem}.muted{color:#bac4d3}figure{margin:0}figcaption{font-weight:650}
@media(prefers-color-scheme:light){body{background:#fff;color:#18202b}.claim,.step-node,.tree-node{background:#f6f8fb;border-color:#ccd3df}.review-status{background:#fff7e8}.step-meta,.muted{color:#49586d}th{background:#edf1f7}}
"#;

fn render_claim_html(claim: &Claim, evidence_index: &BTreeMap<String, usize>) -> String {
    let mut output = format!(
        "<div class=\"claim\"><p class=\"claim-text\">{}</p>{}",
        html_escape(&claim.text),
        render_evidence_html(&claim.evidence, evidence_index)
    );
    if let Some(uncertainty) = claim.uncertainty.as_deref() {
        output.push_str(&format!(
            "<p class=\"claim-uncertainty\"><strong>Uncertainty:</strong> {}</p>",
            html_escape(uncertainty)
        ));
    }
    output.push_str("</div>");
    output
}

fn render_evidence_html(labels: &[String], index: &BTreeMap<String, usize>) -> String {
    if labels.is_empty() {
        return String::new();
    }
    let links = labels
        .iter()
        .map(|label| match index.get(label) {
            Some(number) => format!(
                "<a class=\"citation\" href=\"#evidence-{number}\">{}</a>",
                html_escape(label)
            ),
            None => html_escape(label),
        })
        .collect::<Vec<_>>()
        .join(" ");
    format!("<span class=\"citations\" aria-label=\"Packet evidence\">{links}</span>")
}

fn render_html_steps(
    steps: &[Step],
    evidence_index: &BTreeMap<String, usize>,
    ordered: bool,
) -> String {
    let tag = if ordered { "ol" } else { "ul" };
    let class = if ordered {
        "ordered-steps"
    } else {
        "nested-steps"
    };
    let mut output = format!("<{tag} class=\"{class}\">");
    for step in steps {
        output.push_str("<li><div class=\"step-node\"><span class=\"step-kind\">");
        output.push_str(&html_escape(&step.kind));
        output.push_str("</span><span class=\"step-text\">");
        output.push_str(&html_escape(&step.meaning.text));
        output.push_str("</span>");
        output.push_str(&render_evidence_html(
            &step.meaning.evidence,
            evidence_index,
        ));
        output.push_str(&render_step_metadata_html(step));
        if let Some(uncertainty) = step.meaning.uncertainty.as_deref() {
            output.push_str(&format!(
                "<p class=\"claim-uncertainty\"><strong>Uncertainty:</strong> {}</p>",
                html_escape(uncertainty)
            ));
        }
        output.push_str("</div>");
        if !step.children.is_empty() {
            output.push_str(&format!(
                "<div class=\"path-group\"><div class=\"path-label\">{}</div>{}</div>",
                html_escape(children_label(&step.kind)),
                render_html_steps(&step.children, evidence_index, false)
            ));
        }
        if !step.otherwise.is_empty() {
            output.push_str(&format!(
                "<div class=\"path-group\"><div class=\"path-label\">{}</div>{}</div>",
                html_escape(otherwise_label(&step.kind)),
                render_html_steps(&step.otherwise, evidence_index, false)
            ));
        }
        output.push_str("</li>");
    }
    output.push_str(&format!("</{tag}>"));
    output
}

fn render_step_metadata_html(step: &Step) -> String {
    let mut values = Vec::new();
    for (label, value) in [
        ("From", step.from.as_deref()),
        ("To", step.to.as_deref()),
        ("Interaction", step.interaction.as_deref()),
    ] {
        if let Some(value) = value {
            values.push(format!(
                "<span><strong>{label}:</strong> {}</span>",
                html_escape(value)
            ));
        }
    }
    if values.is_empty() {
        String::new()
    } else {
        format!("<p class=\"step-meta\">{}</p>", values.join(" · "))
    }
}

fn children_label(kind: &str) -> &'static str {
    match kind {
        "decision" => "When the condition holds",
        "try" => "Protected try path",
        "loop" => "Loop body",
        _ => "Following substeps",
    }
}

fn otherwise_label(kind: &str) -> &'static str {
    match kind {
        "try" => "Catch / otherwise path",
        "loop" => "Otherwise / exit path",
        "decision" => "When the condition does not hold",
        _ => "Otherwise path",
    }
}

fn render_tree_html(steps: &[Step], evidence_index: &BTreeMap<String, usize>) -> String {
    if steps.is_empty() {
        return String::from("<p class=\"muted\">No steps supplied.</p>");
    }
    let mut output = String::from("<ul class=\"tree-root\">");
    for step in steps {
        output.push_str("<li><div class=\"tree-node\"><span class=\"step-kind\">");
        output.push_str(&html_escape(&step.kind));
        output.push_str("</span> ");
        output.push_str(&html_escape(&step.meaning.text));
        output.push_str(&render_evidence_html(
            &step.meaning.evidence,
            evidence_index,
        ));
        output.push_str(&render_step_metadata_html(step));
        if !step.children.is_empty() {
            output.push_str(&format!(
                "</div><div class=\"tree-branch-label\">{}</div>{}",
                html_escape(children_label(&step.kind)),
                render_tree_html(&step.children, evidence_index)
            ));
        } else {
            output.push_str("</div>");
        }
        if !step.otherwise.is_empty() {
            output.push_str(&format!(
                "<div class=\"tree-branch-label\">{}</div>{}",
                html_escape(otherwise_label(&step.kind)),
                render_tree_html(&step.otherwise, evidence_index)
            ));
        }
        output.push_str("</li>");
    }
    output.push_str("</ul>");
    output
}

fn render_tree_text(steps: &[Step]) -> String {
    fn append(steps: &[Step], depth: usize, label: Option<&str>, lines: &mut Vec<String>) {
        if let Some(label) = label {
            lines.push(format!("{}[{}]", "  ".repeat(depth), label));
        }
        for (index, step) in steps.iter().enumerate() {
            let branch = if index + 1 == steps.len() {
                "└─"
            } else {
                "├─"
            };
            lines.push(format!(
                "{}{} {}: {}{}",
                "  ".repeat(depth),
                branch,
                step.kind,
                tree_plain_text(&step.meaning.text)
                    .replace('\n', " ")
                    .trim(),
                tree_metadata(step)
            ));
            if !step.children.is_empty() {
                append(
                    &step.children,
                    depth + 1,
                    Some(children_label(&step.kind)),
                    lines,
                );
            }
            if !step.otherwise.is_empty() {
                append(
                    &step.otherwise,
                    depth + 1,
                    Some(otherwise_label(&step.kind)),
                    lines,
                );
            }
        }
    }
    let mut lines = Vec::new();
    append(steps, 0, None, &mut lines);
    lines.join("\n")
}

fn tree_plain_text(value: &str) -> String {
    value.replace("\r", " ").replace('\t', " ")
}

fn tree_metadata(step: &Step) -> String {
    let mut values = Vec::new();
    for (name, value) in [
        ("from", step.from.as_deref()),
        ("to", step.to.as_deref()),
        ("interaction", step.interaction.as_deref()),
    ] {
        if let Some(value) = value {
            values.push(format!(
                "{name}={}",
                tree_plain_text(value).replace('\n', " ")
            ));
        }
    }
    if values.is_empty() {
        String::new()
    } else {
        format!(" ({})", values.join(", "))
    }
}

fn render_markdown_steps(
    steps: &[Step],
    evidence_index: &BTreeMap<String, usize>,
    depth: usize,
    ordered: bool,
) -> String {
    let mut output = String::new();
    let indent = "   ".repeat(depth);
    for (index, step) in steps.iter().enumerate() {
        let prefix = if ordered && depth == 0 {
            format!("{}. ", index + 1)
        } else {
            format!("{}- ", indent)
        };
        output.push_str(&format!(
            "{prefix}**{}:** {}{}{}\n",
            markdown_escape(&step.kind),
            markdown_escape(&step.meaning.text),
            render_evidence_markdown(&step.meaning.evidence, evidence_index),
            markdown_step_metadata(step)
        ));
        if let Some(uncertainty) = step.meaning.uncertainty.as_deref() {
            output.push_str(&format!(
                "{}  - **Uncertainty:** {}\n",
                indent,
                markdown_escape(uncertainty)
            ));
        }
        if !step.children.is_empty() {
            output.push_str(&format!(
                "{}  **{}:**\n",
                indent,
                markdown_escape(children_label(&step.kind))
            ));
            output.push_str(&render_markdown_steps(
                &step.children,
                evidence_index,
                depth + 1,
                false,
            ));
        }
        if !step.otherwise.is_empty() {
            output.push_str(&format!(
                "{}  **{}:**\n",
                indent,
                markdown_escape(otherwise_label(&step.kind))
            ));
            output.push_str(&render_markdown_steps(
                &step.otherwise,
                evidence_index,
                depth + 1,
                false,
            ));
        }
    }
    output
}

fn markdown_step_metadata(step: &Step) -> String {
    let mut values = Vec::new();
    for (label, value) in [
        ("From", step.from.as_deref()),
        ("To", step.to.as_deref()),
        ("Interaction", step.interaction.as_deref()),
    ] {
        if let Some(value) = value {
            values.push(format!("**{label}:** {}", markdown_escape(value)));
        }
    }
    if values.is_empty() {
        String::new()
    } else {
        format!("  ({})", values.join(" · "))
    }
}

fn render_claim_markdown(claim: &Claim, evidence_index: &BTreeMap<String, usize>) -> String {
    let mut output = format!(
        "{}{}",
        markdown_escape(&claim.text),
        render_evidence_markdown(&claim.evidence, evidence_index)
    );
    if let Some(uncertainty) = claim.uncertainty.as_deref() {
        output.push_str(&format!(
            "\n\n**Uncertainty:** {}",
            markdown_escape(uncertainty)
        ));
    }
    output
}

fn render_evidence_markdown(labels: &[String], index: &BTreeMap<String, usize>) -> String {
    if labels.is_empty() {
        return String::new();
    }
    let links = labels
        .iter()
        .map(|label| match index.get(label) {
            Some(number) => format!("[{}](#evidence-{number})", markdown_escape(label)),
            None => markdown_escape(label),
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("  _(Evidence: {links})_ ")
}

fn collect_decisions<'a>(steps: &'a [Step], output: &mut Vec<&'a Step>) {
    for step in steps {
        if step.kind == "decision" {
            output.push(step);
        }
        collect_decisions(&step.children, output);
        collect_decisions(&step.otherwise, output);
    }
}

fn render_decision_tables_html(steps: &[Step], evidence_index: &BTreeMap<String, usize>) -> String {
    let mut decisions = Vec::new();
    collect_decisions(steps, &mut decisions);
    if decisions.is_empty() {
        return String::new();
    }
    let mut output = String::from("<section><h2>Decision tables</h2>");
    for decision in decisions {
        output.push_str(&format!(
            "<table><caption>Decision: {}{}</caption><thead><tr><th>When the condition holds</th><th>When the condition does not hold</th></tr></thead><tbody><tr><td>{}</td><td>{}</td></tr></tbody></table>",
            html_escape(&decision.meaning.text),
            render_evidence_html(&decision.meaning.evidence, evidence_index),
            render_html_steps(&decision.children, evidence_index, true),
            if decision.otherwise.is_empty() {
                "<span class=\"muted\">No otherwise path is supplied in this answer.</span>".into()
            } else {
                render_html_steps(&decision.otherwise, evidence_index, true)
            }
        ));
    }
    output.push_str("</section>");
    output
}

fn render_decision_tables_markdown(
    steps: &[Step],
    evidence_index: &BTreeMap<String, usize>,
) -> String {
    let mut decisions = Vec::new();
    collect_decisions(steps, &mut decisions);
    if decisions.is_empty() {
        return String::new();
    }
    let mut output = String::from("## Decision tables\n\n");
    for decision in decisions {
        output.push_str(&format!(
            "### {}{}\n\n| When the condition holds | When the condition does not hold |\n|---|---|\n| {} | {} |\n\n",
            markdown_escape(&decision.meaning.text),
            render_evidence_markdown(&decision.meaning.evidence, evidence_index),
            markdown_path_cell(&decision.children, evidence_index),
            if decision.otherwise.is_empty() {
                "No otherwise path is supplied in this answer.".into()
            } else {
                markdown_path_cell(&decision.otherwise, evidence_index)
            },
        ));
    }
    output
}

fn markdown_path_cell(steps: &[Step], evidence_index: &BTreeMap<String, usize>) -> String {
    fn append(steps: &[Step], depth: usize, output: &mut Vec<(usize, String, Vec<String>)>) {
        for step in steps {
            output.push((
                depth,
                format!("{}: {}", step.kind, tree_plain_text(&step.meaning.text)),
                step.meaning.evidence.clone(),
            ));
            if !step.children.is_empty() {
                output.push((depth + 1, children_label(&step.kind).into(), Vec::new()));
                append(&step.children, depth + 2, output);
            }
            if !step.otherwise.is_empty() {
                output.push((depth + 1, otherwise_label(&step.kind).into(), Vec::new()));
                append(&step.otherwise, depth + 2, output);
            }
        }
    }
    let mut output = Vec::new();
    append(steps, 0, &mut output);
    if output.is_empty() {
        return "No path is supplied in this answer.".into();
    }
    output
        .into_iter()
        .map(|(depth, text, labels)| {
            format!(
                "{}{}{}",
                "&nbsp;".repeat(depth * 2),
                markdown_table_cell(&text),
                render_evidence_markdown(&labels, evidence_index)
            )
        })
        .collect::<Vec<_>>()
        .join("<br>")
}

fn render_packet_fact_tables_html(
    packet: &Value,
    evidence_index: &BTreeMap<String, usize>,
) -> String {
    let types = packet["types"].as_array().cloned().unwrap_or_default();
    let constants = packet["constants"].as_array().cloned().unwrap_or_default();
    if types.is_empty() && constants.is_empty() {
        return String::new();
    }
    let mut output = String::from("<section><h2>Captured DTO and annotation facts</h2>");
    for dto in types {
        let fields = dto["fields"].as_array().cloned().unwrap_or_default();
        output.push_str(&format!(
            "<table><caption>Type {} · directions {}</caption><thead><tr><th>Field</th><th>Declared type</th><th>Modifiers</th><th>Annotations</th><th>Declaration tokens</th><th>Evidence</th></tr></thead><tbody>",
            html_escape(&value_text(&dto["identity"])),
            html_escape(&value_text(&dto["directions"]))
        ));
        if fields.is_empty() {
            output.push_str("<tr><td colspan=\"6\" class=\"muted\">No field declarations are listed in this packet.</td></tr>");
        }
        for field in fields {
            let labels = combined_labels(&dto["evidence"], &field["evidence"]);
            output.push_str(&format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td><code>{}</code></td><td>{}</td></tr>",
                html_escape(&value_text(&field["name"])),
                html_escape(&value_text(&field["typeDescriptor"])),
                html_escape(&value_text(&field["modifiers"])),
                html_escape(&value_text(&field["annotations"])),
                html_escape(&tokens_text(&field["sourceTokens"])),
                render_evidence_html(&labels, evidence_index)
            ));
        }
        output.push_str("</tbody></table>");
    }
    if !constants.is_empty() {
        output.push_str("<table><caption>Retained constant declarations</caption><thead><tr><th>Owner</th><th>Name</th><th>Declared type</th><th>Modifiers</th><th>Annotations</th><th>Declaration tokens</th><th>Evidence</th></tr></thead><tbody>");
        for constant in constants {
            output.push_str(&format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td><code>{}</code></td><td>{}</td></tr>",
                html_escape(&value_text(&constant["ownerIdentity"])),
                html_escape(&value_text(&constant["name"])),
                html_escape(&value_text(&constant["typeDescriptor"])),
                html_escape(&value_text(&constant["modifiers"])),
                html_escape(&value_text(&constant["annotations"])),
                html_escape(&tokens_text(&constant["sourceTokens"])),
                render_evidence_html(&strings(&constant["evidence"]), evidence_index)
            ));
        }
        output.push_str("</tbody></table>");
    }
    output.push_str("</section>");
    output
}

fn render_packet_fact_tables_markdown(
    packet: &Value,
    evidence_index: &BTreeMap<String, usize>,
) -> String {
    let types = packet["types"].as_array().cloned().unwrap_or_default();
    let constants = packet["constants"].as_array().cloned().unwrap_or_default();
    if types.is_empty() && constants.is_empty() {
        return String::new();
    }
    let mut output = String::from("## Captured DTO and annotation facts\n\n");
    for dto in types {
        output.push_str(&format!(
            "### Type {} · directions {}\n\n| Field | Declared type | Modifiers | Annotations | Declaration tokens | Evidence |\n|---|---|---|---|---|---|\n",
            markdown_escape(&value_text(&dto["identity"])),
            markdown_escape(&value_text(&dto["directions"]))
        ));
        let fields = dto["fields"].as_array().cloned().unwrap_or_default();
        if fields.is_empty() {
            output
                .push_str("| No field declarations are listed in this packet. |  |  |  |  |  |\n");
        }
        for field in fields {
            let labels = combined_labels(&dto["evidence"], &field["evidence"]);
            output.push_str(&format!(
                "| {} | {} | {} | {} | {} | {} |\n",
                markdown_table_cell(&value_text(&field["name"])),
                markdown_table_cell(&value_text(&field["typeDescriptor"])),
                markdown_table_cell(&value_text(&field["modifiers"])),
                markdown_table_cell(&value_text(&field["annotations"])),
                markdown_code_cell(&tokens_text(&field["sourceTokens"])),
                markdown_evidence_cell(&labels, evidence_index)
            ));
        }
        output.push('\n');
    }
    if !constants.is_empty() {
        output.push_str("### Retained constant declarations\n\n| Owner | Name | Declared type | Modifiers | Annotations | Declaration tokens | Evidence |\n|---|---|---|---|---|---|---|\n");
        for constant in constants {
            output.push_str(&format!(
                "| {} | {} | {} | {} | {} | {} | {} |\n",
                markdown_table_cell(&value_text(&constant["ownerIdentity"])),
                markdown_table_cell(&value_text(&constant["name"])),
                markdown_table_cell(&value_text(&constant["typeDescriptor"])),
                markdown_table_cell(&value_text(&constant["modifiers"])),
                markdown_table_cell(&value_text(&constant["annotations"])),
                markdown_code_cell(&tokens_text(&constant["sourceTokens"])),
                markdown_evidence_cell(&strings(&constant["evidence"]), evidence_index)
            ));
        }
        output.push('\n');
    }
    output
}

fn render_packet_limits_html(packet: &Value) -> String {
    let limits = packet["limitations"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let interpretation = packet["interpretationLimits"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let endpoint_boundaries = packet["endpoint"]["boundaries"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let coverage_boundaries = packet["coverage"]["boundaries"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let runtime = packet.get("runtimeAndSerialization");
    if limits.is_empty()
        && interpretation.is_empty()
        && endpoint_boundaries.is_empty()
        && coverage_boundaries.is_empty()
        && runtime.is_none()
    {
        return String::new();
    }
    let mut output = String::from("<section class=\"limitations\"><h2>Packet gaps and limits</h2>");
    append_html_value_list(&mut output, "Captured gaps", &limits);
    append_html_value_list(&mut output, "Interpretation limits", &interpretation);
    append_html_value_list(&mut output, "Endpoint boundaries", &endpoint_boundaries);
    append_html_value_list(&mut output, "Coverage boundaries", &coverage_boundaries);
    if let Some(runtime) = runtime {
        output.push_str(&format!(
            "<p><strong>Runtime and serialization:</strong> {}</p>",
            html_escape(&value_text(runtime))
        ));
    }
    output.push_str("</section>");
    output
}

fn append_html_value_list(output: &mut String, title: &str, values: &[Value]) {
    if values.is_empty() {
        return;
    }
    output.push_str(&format!(
        "<h3>{}</h3><ul class=\"limitations\">",
        html_escape(title)
    ));
    for value in values {
        output.push_str(&format!(
            "<li><code>{}</code></li>",
            html_escape(&value_text(value))
        ));
    }
    output.push_str("</ul>");
}

fn render_packet_limits_markdown(packet: &Value) -> String {
    let limits = packet["limitations"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let interpretation = packet["interpretationLimits"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let endpoint_boundaries = packet["endpoint"]["boundaries"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let coverage_boundaries = packet["coverage"]["boundaries"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let runtime = packet.get("runtimeAndSerialization");
    if limits.is_empty()
        && interpretation.is_empty()
        && endpoint_boundaries.is_empty()
        && coverage_boundaries.is_empty()
        && runtime.is_none()
    {
        return String::new();
    }
    let mut output = String::from("## Packet gaps and limits\n\n");
    append_markdown_value_list(&mut output, "Captured gaps", &limits);
    append_markdown_value_list(&mut output, "Interpretation limits", &interpretation);
    append_markdown_value_list(&mut output, "Endpoint boundaries", &endpoint_boundaries);
    append_markdown_value_list(&mut output, "Coverage boundaries", &coverage_boundaries);
    if let Some(runtime) = runtime {
        output.push_str(&format!(
            "**Runtime and serialization:** {}\n\n",
            markdown_escape(&value_text(runtime))
        ));
    }
    output
}

fn append_markdown_value_list(output: &mut String, title: &str, values: &[Value]) {
    if values.is_empty() {
        return;
    }
    output.push_str(&format!("### {}\n\n", markdown_escape(title)));
    for value in values {
        output.push_str(&format!("- `{}`\n", markdown_escape(&value_text(value))));
    }
    output.push('\n');
}

fn render_uncertainties_html(answer: &OperationAnswer) -> String {
    if answer.uncertainties.is_empty() {
        return String::new();
    }
    let mut output = String::from("<section><h2>Uncertainties</h2><ul class=\"uncertainties\">");
    for uncertainty in &answer.uncertainties {
        output.push_str(&format!("<li>{}</li>", html_escape(uncertainty)));
    }
    output.push_str("</ul></section>");
    output
}

fn render_uncertainties_markdown(answer: &OperationAnswer) -> String {
    if answer.uncertainties.is_empty() {
        return String::new();
    }
    let mut output = String::from("## Uncertainties\n\n");
    for uncertainty in &answer.uncertainties {
        output.push_str(&format!("- {}\n", markdown_escape(uncertainty)));
    }
    output.push('\n');
    output
}

fn render_evidence_index_html(
    citations: &serde_json::Map<String, Value>,
    evidence_index: &BTreeMap<String, usize>,
) -> String {
    if evidence_index.is_empty() {
        return String::new();
    }
    let mut output =
        String::from("<section><h2>Packet evidence index</h2><ol class=\"evidence-index\">");
    for (label, number) in evidence_index {
        output.push_str(&format!(
            "<li id=\"evidence-{number}\"><strong>{}</strong> — {}</li>",
            html_escape(label),
            html_escape(&value_text(&citations[label]))
        ));
    }
    output.push_str("</ol></section>");
    output
}

fn render_evidence_index_markdown(
    citations: &serde_json::Map<String, Value>,
    evidence_index: &BTreeMap<String, usize>,
) -> String {
    if evidence_index.is_empty() {
        return String::new();
    }
    let mut output = String::from("## Packet evidence index\n\n");
    for (label, number) in evidence_index {
        output.push_str(&format!(
            "<a id=\"evidence-{number}\"></a>- **{}** — {}\n",
            markdown_escape(label),
            markdown_escape(&value_text(&citations[label]))
        ));
    }
    output.push('\n');
    output
}

fn combined_labels(first: &Value, second: &Value) -> Vec<String> {
    let mut seen = BTreeSet::new();
    strings(first)
        .into_iter()
        .chain(strings(second))
        .filter(|label| seen.insert(label.clone()))
        .collect()
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

fn value_text(value: &Value) -> String {
    match value {
        Value::Null => "—".into(),
        Value::String(text) => text.clone(),
        Value::Array(values) => values.iter().map(value_text).collect::<Vec<_>>().join(", "),
        Value::Object(_) => compact_json(value),
        _ => value.to_string(),
    }
}

fn compact_json(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "[unavailable]".into())
}

fn markdown_tree_block(steps: &[Step]) -> String {
    let tree = render_tree_text(steps);
    let longest_backtick_run = tree
        .split(|character| character != '`')
        .map(str::len)
        .max()
        .unwrap_or(0);
    let fence = "`".repeat(3.max(longest_backtick_run + 1));
    format!("{fence}text\n{tree}\n{fence}\n")
}

fn markdown_table_cell(value: &str) -> String {
    markdown_escape(&value.replace(['\r', '\n'], " "))
}

fn markdown_code_cell(value: &str) -> String {
    if value == "—" {
        return value.into();
    }
    let value = value
        .replace(['\r', '\n'], " ")
        .replace('|', "\\|")
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    let longest_run = value
        .split(|character| character != '`')
        .map(str::len)
        .max()
        .unwrap_or(0);
    let fence = "`".repeat(longest_run + 1);
    format!("{fence}{value}{fence}")
}

fn markdown_evidence_cell(labels: &[String], index: &BTreeMap<String, usize>) -> String {
    if labels.is_empty() {
        return "—".into();
    }
    labels
        .iter()
        .map(|label| match index.get(label) {
            Some(number) => format!("[{}](#evidence-{number})", markdown_escape(label)),
            None => markdown_escape(label),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn tokens_text(value: &Value) -> String {
    let Some(tokens) = value.as_array() else {
        return "—".into();
    };
    tokens
        .iter()
        .map(|token| {
            token
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| value_text(token))
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn markdown_escape(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\\' | '`' | '*' | '_' | '{' | '}' | '[' | ']' | '(' | ')' | '#' | '+' | '-' | '.'
            | '!' | '|' | '~' => {
                output.push('\\');
                output.push(character);
            }
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '&' => output.push_str("&amp;"),
            '\r' => {}
            '\n' => output.push(' '),
            _ => output.push(character),
        }
    }
    output
}

fn html_escape(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            '\'' => output.push_str("&#39;"),
            _ => output.push(character),
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn packet() -> Value {
        let mut packet = json!({
            "schema":PACKET_SCHEMA,
            "profile":"endpoint-context-v3",
            "documentationLanguage":"en",
            "audience":"API maintainers",
            "title":"Transfer request",
            "citations":{
                "d1":"retained handler and declaration facts",
                "p1":"captured endpoint outline"
            },
            "endpoint":{"symbol":"api.Transfer.handle","trigger":"POST /transfer","boundaries":[],"evidence":["p1"]},
            "types":[{
                "identity":"api.TransferRequest",
                "directions":["INPUT"],
                "evidence":["d1"],
                "fields":[{
                    "name":"count",
                    "typeDescriptor":"int",
                    "modifiers":["PRIVATE"],
                    "annotations":[{"name":"Min","value":0}],
                    "sourceTokens":["@Min","(","0",")","int","count"],
                    "evidence":["d1"]
                }]
            }],
            "constants":[{
                "ownerIdentity":"api.TransferPolicy",
                "name":"MAX_TRIES",
                "typeDescriptor":"int",
                "modifiers":["STATIC","FINAL"],
                "annotations":[],
                "sourceTokens":["static","final","int","MAX_TRIES","=","7"],
                "evidence":["d1"]
            }],
            "callMap":{"authority":"RETAINED_TARGET_RELATIONS","order":"NOT_EXECUTION_ORDER","nodes":[],"edges":[{"evidence":["d1"]}]},
            "methodBodies":[{"id":"b1","node":"m0","source":"private void handle() {}","sourceAuthority":"CAPTURED_SOURCE","text":"private void handle() {}","evidence":["d1"]}],
            "coverage":{"coverage":"PARTIAL","runtimeMode":"SOURCE_ONLY","boundaries":[],"callAuthority":"SYNTAX_UNRESOLVED","evidence":["p1"]},
            "limitations":[{"code":"CALL_TARGET_BODY_NOT_CAPTURED","count":2}],
            "interpretationLimits":["Call relations do not establish runtime execution."],
            "runtimeAndSerialization":"UNKNOWN_FROM_THIS_PACKET"
        });
        let packet_digest = crate::documentation::digest(&packet).unwrap();
        packet["packetDigest"] = json!(packet_digest);
        packet
    }

    fn answer(packet: &Value) -> Value {
        json!({
            "schema":ANSWER_SCHEMA,
            "packetDigest":packet["packetDigest"],
            "title":"Transfer request handling",
            "summary":{"text":"The captured endpoint delegates to the retained handler.","evidence":["p1","d1"]},
            "steps":[
                {
                    "kind":"decision",
                    "meaning":{"text":"The handler checks whether the request is valid.","evidence":["d1"]},
                    "children":[{
                        "kind":"action",
                        "meaning":{"text":"Continue with the accepted request.","evidence":["d1"]}
                    }],
                    "otherwise":[{
                        "kind":"try",
                        "meaning":{"text":"The handler makes the upstream request.","evidence":["d1"]},
                        "children":[{
                            "kind":"decision",
                            "meaning":{"text":"The upstream response is present.","evidence":["d1"]},
                            "children":[{
                                "kind":"return",
                                "meaning":{"text":"Return the response.","evidence":["d1"]}
                            }],
                            "otherwise":[{
                                "kind":"throw",
                                "meaning":{"text":"Raise for the missing response.","evidence":["d1"]}
                            }]
                        }],
                        "otherwise":[{
                            "kind":"throw",
                            "meaning":{"text":"Translate the caught failure.","evidence":["d1"]}
                        }]
                    }]
                }
            ],
            "uncertainties":[]
        })
    }

    #[test]
    fn one_tree_preserves_decision_and_try_catch_paths_in_each_view() {
        let packet = packet();
        let rendered = validate_and_render(&packet, answer(&packet)).unwrap();

        assert!(rendered.html.contains("When the condition holds"));
        assert!(rendered.html.contains("When the condition does not hold"));
        assert!(rendered.html.contains("Protected try path"));
        assert!(rendered.html.contains("Catch / otherwise path"));
        assert_eq!(
            rendered.html.matches("<table><caption>Decision:").count(),
            2
        );
        let decision_tables = rendered
            .markdown
            .split("## Decision tables\n\n")
            .nth(1)
            .unwrap()
            .split("## Captured DTO and annotation facts")
            .next()
            .unwrap();
        assert_eq!(
            decision_tables
                .matches("| When the condition holds |")
                .count(),
            2
        );
        assert!(decision_tables.contains("When the condition does not hold"));
        assert!(decision_tables.contains("Protected try path"));
        assert!(decision_tables.contains("Catch / otherwise path"));
    }

    #[test]
    fn digest_evidence_labels_nonempty_steps_and_checks_are_validated() {
        let packet = packet();
        let valid = answer(&packet);

        let mut wrong_digest = valid.clone();
        wrong_digest["packetDigest"] = json!("sha256:wrong");
        assert!(validate_and_render(&packet, wrong_digest).is_err());

        let mut unknown_label = valid.clone();
        unknown_label["summary"]["evidence"] = json!(["missing"]);
        assert!(validate_and_render(&packet, unknown_label).is_err());

        let mut empty_steps = valid.clone();
        empty_steps["steps"] = json!([]);
        assert!(validate_and_render(&packet, empty_steps).is_err());

        let mut authored_check = valid;
        authored_check["summary"]["checks"] = json!([{
            "kind":"factEquals",
            "evidence":"d1",
            "field":"name",
            "expected":"handle"
        }]);
        assert!(validate_and_render(&packet, authored_check).is_err());
    }

    #[test]
    fn packet_limitations_and_fact_tables_are_rendered_without_reauthoring() {
        let packet = packet();
        let rendered = validate_and_render(&packet, answer(&packet)).unwrap();

        assert!(rendered.html.contains("CALL_TARGET_BODY_NOT_CAPTURED"));
        assert!(rendered.html.contains("UNKNOWN_FROM_THIS_PACKET"));
        assert!(rendered.html.contains("&quot;value&quot;:0"));
        assert!(rendered.html.contains("@Min ( 0 ) int count"));
        assert!(rendered.html.contains("MAX_TRIES = 7"));
        assert!(rendered.html.contains("api.TransferPolicy"));

        // Markdown escapes punctuation in prose cells, while declaration tokens
        // are retained as code and remain verbatim.
        assert!(
            rendered
                .markdown
                .contains("CALL\\_TARGET\\_BODY\\_NOT\\_CAPTURED")
        );
        assert!(rendered.markdown.contains("UNKNOWN\\_FROM\\_THIS\\_PACKET"));
        assert!(rendered.markdown.contains("\"value\":0"));
        assert!(rendered.markdown.contains("@Min ( 0 ) int count"));
        assert!(rendered.markdown.contains("MAX_TRIES = 7"));
        assert!(rendered.markdown.contains("api\\.TransferPolicy"));
    }

    #[test]
    fn all_untrusted_html_text_is_escaped() {
        let mut packet = packet();
        packet["citations"]["d1"] = json!("role <img src=x onerror=alert(1)>");
        packet.as_object_mut().unwrap().remove("packetDigest");
        let packet_digest = crate::documentation::digest(&packet).unwrap();
        packet["packetDigest"] = json!(packet_digest);

        let mut answer = answer(&packet);
        answer["title"] = json!("<script>alert(1)</script>");
        answer["summary"]["text"] = json!("Use <b>captured</b> evidence.");
        let rendered = validate_and_render(&packet, answer).unwrap();

        assert!(!rendered.html.contains("<script>"));
        assert!(!rendered.html.contains("<img src=x"));
        assert!(
            rendered
                .html
                .contains("&lt;script&gt;alert(1)&lt;/script&gt;")
        );
        assert!(rendered.html.contains("&lt;img src=x onerror=alert(1)&gt;"));
    }
}
