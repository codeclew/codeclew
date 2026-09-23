//! Convert a method's structured flow events into a PlantUML activity diagram.
//! Evidence binding: the root symbol is emitted as an evidence comment; each
//! step is rendered as an activity action or branch.

use serde_json::Value;

/// Render a flow event array into PlantUML activity body.
/// Returns `None` if the flow has no usable steps.
pub fn activity_from_flow(events: &Value, symbol: &str) -> Option<String> {
    let rows = events.as_array()?;
    if rows.is_empty() {
        return None;
    }
    let mut out = String::new();
    out.push_str(&format!("' evidence: {}\n", symbol));
    for (i, row) in rows.iter().enumerate() {
        let kind = row["kind"].as_str().unwrap_or("STATEMENT");
        let text = row["text"].as_str().unwrap_or("");
        match kind {
            "IF" => out.push_str(&format!("if ({}) then (да)\n", text)),
            "ELSE" | "ELSEIF" => out.push_str("else (нет)\n"),
            "END_IF" | "ENDIF" => out.push_str("endif\n"),
            "LOOP" => out.push_str(&format!("while ({}) is (да)\n", text)),
            "END_LOOP" => out.push_str("endwhile\n"),
            "RETURN" => {
                out.push_str(&format!(":{}\n", text));
                if i + 1 == rows.len() {
                    out.push_str("stop\n");
                }
            }
            "THROW" => out.push_str(&format!(":throw {};\n", text)),
            "BOUNDARY" => out.push_str(&format!(
                "note right\n  {} (не развёрнуто)\nend note\n",
                text
            )),
            _ => out.push_str(&format!(":{};\n", text)), // STATEMENT, CALL, LOCAL
        }
    }
    if !out.contains("stop") {
        out.push_str("stop\n");
    }
    Some(out)
}

/// Produce a full PlantUML activity document for a flow, or `None` if empty.
pub fn document(events: &Value, symbol: &str, title: &str) -> Option<String> {
    let body = activity_from_flow(events, symbol)?;
    Some(format!(
        "@startuml\n!theme plain\n!pragma layout smetana\ntitle {}\nstart\n{body}\n@enduml\n",
        super::plantuml::escape(title)
    ))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    #[test]
    fn flow_events_become_activity_steps() {
        let events = json!([
            {"kind":"STATEMENT","text":"TaskInstanceDao taskInstance"},
            {"kind":"IF","text":"Optional.of(request).map(getAnyTask).orElse(false)"},
            {"kind":"CALL","text":"taskHandlingService.changeAnyTaskStatus(...)"},
            {"kind":"RETURN","text":"createResponse(...)"}
        ]);
        let src = "method:class:TaskService#changeTaskStatus(JL...)Z";
        let body = super::activity_from_flow(&events, src).unwrap();
        assert!(body.contains(":TaskInstanceDao taskInstance;"));
        assert!(body.contains("if (Optional.of(request).map(getAnyTask).orElse(false)) then (да)"));
        assert!(body.contains(":taskHandlingService.changeAnyTaskStatus(...);"));
        assert!(body.contains("stop"));
        assert!(body.contains("' evidence: method:class:TaskService#changeTaskStatus(JL...)Z"));
    }

    #[test]
    fn empty_flow_returns_none() {
        let events = json!([]);
        assert!(super::activity_from_flow(&events, "s").is_none());
    }
}
