//! Standalone presentation of an already-validated frozen Narrative, never an answer.
use super::{invalid, reader_presentation, render::escape};
use crate::error::ClewError;
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Debug)]
struct Step<'a> {
    event: &'a Value,
    children: Vec<Step<'a>>,
    alternatives: Vec<Step<'a>>,
}
fn steps<'a>(events: &'a [Value], cursor: &mut usize, nested: bool) -> Vec<Step<'a>> {
    let mut result = Vec::new();
    while let Some(event) = events.get(*cursor) {
        let kind = text(event, "kind");
        if nested && matches!(kind, "end" | "else") {
            break;
        }
        *cursor += 1;
        if kind == "end" {
            continue;
        }
        let mut step = Step {
            event,
            children: vec![],
            alternatives: vec![],
        };
        if matches!(kind, "alt" | "loop" | "opt") {
            step.children = steps(events, cursor, true);
            while events
                .get(*cursor)
                .is_some_and(|e| text(e, "kind") == "else")
            {
                let marker = &events[*cursor];
                *cursor += 1;
                step.alternatives.push(Step {
                    event: marker,
                    children: steps(events, cursor, true),
                    alternatives: vec![],
                });
            }
            if events
                .get(*cursor)
                .is_some_and(|e| text(e, "kind") == "end")
            {
                *cursor += 1;
            }
        }
        result.push(step);
    }
    result
}
fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or_default()
}
fn refs(value: &Value) -> Vec<&str> {
    value["sourceIds"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect()
}
fn citations(value: &Value, index: &BTreeMap<String, usize>) -> String {
    let links = refs(value)
        .iter()
        .filter_map(|id| {
            index.get(*id).map(|number| {
                format!(
                    "<a class=\"citation\" href=\"#source-{}\">E{number}</a>",
                    escape(id)
                )
            })
        })
        .collect::<Vec<_>>();
    if links.is_empty() {
        String::new()
    } else {
        reader_presentation::citations(&format!("Evidence ({})", links.len()), &links.join(" "))
    }
}
fn claim(value: &Value, index: &BTreeMap<String, usize>) -> String {
    format!(
        "<div class=\"claim\"><p class=\"claim-text\">{}</p>{}</div>",
        escape(text(value, "text")),
        citations(value, index)
    )
}
fn step_list(
    nodes: &[Step<'_>],
    index: &BTreeMap<String, usize>,
    nested: bool,
    external: &[Value],
) -> String {
    let (tag, class) = if nested {
        ("ul", "nested-steps")
    } else {
        ("ol", "ordered-steps")
    };
    let mut out = format!("<{tag} class=\"{class}\">");
    for node in nodes {
        out.push_str(&super::operation_answer::step_node_open(
            &format!("reader-step-{}", text(node.event, "id")),
            Some(text(node.event, "kind")),
        ));
        out.push_str(&format!(
            "<span class=\"step-text\">{}</span>{}",
            escape(text(node.event, "text")),
            citations(node.event, index)
        ));
        for row in external
            .iter()
            .filter(|row| row["eventId"] == node.event["id"])
        {
            out.push_str(&format!("<details class=\"external-call-details\"><summary>{}</summary><pre>{}</pre></details>",escape(text(row,"title")),escape(&serde_json::to_string_pretty(row).unwrap_or_default())));
        }
        out.push_str("</div>");
        if !node.children.is_empty() {
            out.push_str(&format!(
                "<div class=\"path-group\"><div class=\"path-label\">{}</div>{}</div>",
                if text(node.event, "kind") == "alt" {
                    "If the condition holds"
                } else {
                    "Within this group"
                },
                step_list(&node.children, index, true, external)
            ));
        }
        for arm in &node.alternatives {
            out.push_str(&format!(
                "<div class=\"path-group\" id=\"reader-step-{}\"><div class=\"path-label\">{}</div>{}{}</div>",
                escape(text(arm.event,"id")), escape(text(arm.event,"text")), citations(arm.event,index),
                step_list(&arm.children, index, true, external)
            ));
        }
        out.push_str("</li>");
    }
    out.push_str(&format!("</{tag}>"));
    out
}

pub(super) fn html(data: &Value) -> Result<String, ClewError> {
    let id = text(&data["readerPresentation"], "operationId");
    let operations = data["operations"]
        .as_array()
        .ok_or_else(|| invalid("frozen reader requires operations"))?;
    let op = operations
        .iter()
        .find(|o| text(o, "id") == id)
        .ok_or_else(|| invalid("selected frozen operation is absent"))?;
    let overview = operations
        .iter()
        .find(|o| text(o, "id") == "process-overview");
    let sources = data["operationSources"][id]
        .as_object()
        .ok_or_else(|| invalid("frozen operation source records are absent"))?;
    let index: BTreeMap<String, usize> = sources
        .keys()
        .enumerate()
        .map(|(i, id)| (id.clone(), i + 1))
        .collect();
    let events = op["events"]
        .as_array()
        .ok_or_else(|| invalid("frozen operation event records are absent"))?;
    let tree = steps(events, &mut 0, false);
    let mut body = format!(
        "<h1>{}</h1><p class=\"review-status\"><strong>Frozen Narrative · {}</strong> — source freshness: {}. Runtime execution is unverified.</p>",
        escape(text(op, "title")),
        escape(text(&data["sectionState"], "verification")),
        escape(text(&data["sectionState"], "freshness"))
    );
    body.push_str("<nav class=\"document-nav\" aria-label=\"Document navigation\"><a href=\"#summary\">Behavior explanation</a> · <a href=\"#process-outline\">Process outline</a> · <a href=\"#glossary\">Domain entities</a> · <a href=\"#ordered-behavior\">Ordered behavior</a></nav><details class=\"document-nav-details\"><summary>Details and evidence</summary><nav class=\"document-nav\" aria-label=\"Detail navigation\"><a href=\"#source-process-projection\">Sequence diagram</a> · <a href=\"#preparations\">Preparations</a> · <a href=\"#semantic-predicates\">Predicate details</a> · <a href=\"#cited-evidence\">Cited evidence</a> · <a href=\"#evidence-contexts\">Evidence contexts</a></nav></details>");
    body.push_str(&format!("<section id=\"summary\" class=\"behavior-summary\"><h2>Behavior explanation</h2>{}</section><section id=\"process-outline\"><h2>Process outline</h2><ol class=\"process-outline\">",claim(&op["summary"],&index)));
    for node in &tree {
        body.push_str(&format!(
            "<li><a href=\"#reader-step-{}\"><span class=\"step-kind\">{}</span>{}</a></li>",
            escape(text(node.event, "id")),
            escape(text(node.event, "kind")),
            escape(text(node.event, "text"))
        ));
    }
    body.push_str("</ol><div class=\"outline-text\">");
    if let Some(overview) = overview {
        body.push_str(&claim(&overview["summary"], &index));
    }
    body.push_str("<p class=\"muted\">The cards project retained top-level Narrative events. The source-derived outline has separate authority.</p></div></section><section><h2>Uncertainties and gaps</h2><ul class=\"uncertainties\">");
    if data["processOutline"]["status"] == "GAP" {
        body.push_str(&format!(
            "<li>Source-derived outline: <code>{}</code>.</li>",
            escape(text(&data["processOutline"], "gap"))
        ));
    }
    body.push_str(&format!("<li>Source freshness: {}. Meaning review: {}.</li><li>Selected source evidence does not reverify the retained narrative or runtime behavior.</li></ul></section><section id=\"glossary\"><h2>Domain entities</h2><div class=\"glossary-list\"><article class=\"glossary-term\"><h3>No retained glossary definitions</h3><div class=\"claim\"><p class=\"claim-text muted\">This frozen Narrative has no accepted glossary definitions. Entity meanings are not inferred from participant or source names.</p></div></article></div></section><section id=\"ordered-behavior\"><h2>Ordered behavior</h2>{}</section>",escape(text(&data["sectionState"],"freshness")),escape(text(&data["sectionState"],"verification")),step_list(&tree,&index,false,op_external(data,id))));
    body.push_str("<details class=\"implementation-details\"><summary>Implementation details and retained commentary</summary>");
    for operation in operations {
        for fragment in operation["explanation"]
            .as_array()
            .into_iter()
            .flatten()
            .chain(operation["findings"].as_array().into_iter().flatten())
        {
            body.push_str(&claim(fragment, &index));
        }
    }
    body.push_str("<section id=\"semantic-predicates\"><h3>Predicate details</h3><p class=\"muted\">No accepted semantic predicate definitions are present in this Narrative. Original branch conditions remain in ordered behavior.</p></section><section id=\"preparations\"><h3>Preparations</h3><p class=\"muted\">No separate accepted preparation blocks are present in this Narrative.</p></section><section id=\"source-process-projection\"><h2>Supplementary sequence diagram</h2><details><summary>Open the retained Narrative sequence</summary><p class=\"muted\">Projection of retained message steps. Notes remain in ordered behavior; runtime execution is unverified.</p><div id=\"sequence-host\"></div><div id=\"diagram-selected-evidence\"></div></details></section></details><details class=\"technical-details\"><summary>Technical details and evidence</summary><details class=\"evidence-reference\"><summary>Original source evidence</summary><section id=\"cited-evidence\" class=\"source-locations\"><h2>Cited evidence</h2>");
    for (source_id, source) in sources {
        body.push_str(&format!("<details id=\"source-{}\"><summary>E{} · {} / {}:{}–{}</summary><p><code>{}</code> · {}</p><p>Text: <code>{}</code><br>Evidence: <code>{}</code></p>",escape(source_id),index[source_id],escape(text(source,"service")),escape(text(source,"file")),source["startLine"],source["endLine"],escape(text(source,"revision")),escape(text(source,"authority")),escape(text(source,"textDigest")),escape(text(source,"evidenceDigest"))));
        if source["url"]
            .as_str()
            .is_some_and(|u| u.starts_with("https://") || u.starts_with("http://"))
        {
            body.push_str(&format!(
                "<p><a href=\"{}\">Original source</a></p>",
                escape(text(source, "url"))
            ));
        }
        let mut binding = source.clone();
        binding.as_object_mut().unwrap().remove("text");
        body.push_str(&format!(
            "<details><summary>Complete source binding</summary><pre>{}</pre></details>",
            escape(&serde_json::to_string_pretty(&binding).map_err(super::io_error)?)
        ));
        body.push_str(&format!(
            "<pre tabindex=\"0\">{}</pre></details>",
            escape(text(source, "text"))
        ));
    }
    body.push_str("</section></details><section id=\"evidence-contexts\"><h3>Evidence contexts and frozen provenance</h3>");
    let provenance = json!({"selectedSnapshot":data["selectedSnapshot"],"selectedEvidence":data["selectedEvidence"],"frozenPublication":data["frozenPublication"],"sectionState":data["sectionState"],"sourceAuthorities":data["sourceAuthorities"],"revisions":data["revisions"],"processOutline":data["processOutline"],"boundaries":op["boundaries"],"interactions":data["interactions"]});
    body.push_str(&format!(
        "<pre>{}</pre></section></details>",
        escape(&serde_json::to_string_pretty(&provenance).map_err(super::io_error)?)
    ));
    let payload = serde_json::to_string(data)
        .map_err(super::io_error)?
        .replace('<', "\\u003c")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029");
    Ok(format!(
        "<!doctype html><html lang=\"{}\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{}</title><style>{}</style><style>{}</style></head><body><main>{body}</main><script type=\"application/json\" id=\"document-data\">{payload}</script><script>{}\n{}</script></body></html>",
        escape(text(data, "language")),
        escape(text(op, "title")),
        reader_presentation::STYLE,
        include_str!("../../assets/documentation/operation-reader-extras.css"),
        include_str!("../../assets/documentation/operation-reader.js"),
        include_str!("../../assets/documentation/sequence.js")
    ))
}
fn op_external<'a>(data: &'a Value, id: &str) -> &'a [Value] {
    data["operationExternalCalls"][id]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn standalone_template_keeps_nested_arms_exact_sources_and_truthful_gaps() {
        let data = json!({"language":"en","readerPresentation":{"operationId":"consume"},
            "operations":[{"id":"consume","title":"Consume","summary":{"text":"Exact summary","sourceIds":["s"]},"events":[
                {"id":"guard","kind":"alt","text":"If <unsafe>","sourceIds":["s"]},
                {"id":"yes","kind":"note","text":"Yes text","sourceIds":["s"]},
                {"id":"else","kind":"else","text":"Otherwise","sourceIds":["s"]},
                {"id":"no","kind":"note","text":"No text","sourceIds":["s"]},
                {"id":"end","kind":"end","text":"","sourceIds":["s"]}],
                "explanation":[{"text":"Original commentary","sourceIds":["s"]}],"findings":[{"text":"Original finding","sourceIds":["s"]}]}],
            "sectionState":{"freshness":"STALE","verification":"UNASSESSED"},"processOutline":{"status":"GAP","gap":"PROCESS_EVIDENCE_CHANGED_REVIEW_REQUIRED"},
            "operationSources":{"consume":{"s":{"service":"synthetic","file":"Example.ts","startLine":1,"endLine":2,"revision":"original-revision","authority":"EXACT_SNAPSHOT_TEXT","text":"one\n<two>","textDigest":"text-digest","evidenceDigest":"evidence-digest"}}}});
        let before = data.clone();
        let rendered = html(&data).unwrap();
        assert_eq!(data, before);
        for marker in [
            "class=\"behavior-summary\"",
            "class=\"document-nav-details\"",
            "class=\"process-outline\"",
            "class=\"ordered-steps\"",
            "class=\"nested-steps\"",
            "class=\"path-group\"",
            "class=\"citations\"",
            "class=\"citation\"",
            "class=\"glossary-term\"",
            "Original commentary",
            "Original finding",
            "PROCESS_EVIDENCE_CHANGED_REVIEW_REQUIRED",
            "one\n&lt;two&gt;",
            "text-digest",
            "evidence-digest",
        ] {
            assert!(rendered.contains(marker), "{marker}");
        }
        assert!(rendered.contains("If &lt;unsafe&gt;"));
        let order = [
            "<h1>",
            "class=\"review-status\"",
            "<nav class=\"document-nav\"",
            "class=\"document-nav-details\"",
            "id=\"summary\"",
            "id=\"process-outline\"",
            "id=\"glossary\"",
            "id=\"ordered-behavior\"",
            "class=\"implementation-details\"",
            "class=\"technical-details\"",
        ];
        let positions = order.map(|m| rendered.find(m).unwrap());
        assert!(positions.windows(2).all(|p| p[0] < p[1]));
        assert!(!rendered.contains("class=\"sidebar\""));
        assert!(!rendered.contains("codeclew-operation-answer"));
        assert!(!rendered.contains("MODEL_APPROVED"));
        let tree = steps(
            data["operations"][0]["events"].as_array().unwrap(),
            &mut 0,
            false,
        );
        assert_eq!(tree.len(), 1);
        assert_eq!(text(tree[0].children[0].event, "id"), "yes");
        assert_eq!(text(tree[0].alternatives[0].children[0].event, "id"), "no");
    }
}
