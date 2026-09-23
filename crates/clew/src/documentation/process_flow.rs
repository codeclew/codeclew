//! Convert a method's structured flow events into a PlantUML activity diagram.
//! Evidence binding: the root symbol is emitted as an evidence comment; each
//! step is rendered as an activity action or branch.
//!
//! Real retained FLOW events use the schema below (no `text` field; call
//! targets are method symbols, branches carry a `condition`):
//! - `CALL` / `CONSTRUCT`: `target` is a `method:class:...#name(...)desc` symbol
//! - `IF` / `LOOP`: descriptive text is in `condition`
//! - `ELSE`, `RETURN`, `THROW`, `STATEMENT`, `LOCAL`, `BOUNDARY`: no text
//! - `END`: closes the nearest open `IF` or `LOOP`

use serde_json::Value;

/// Shorten a `method:class:<Owner>#<name>(...)<desc>` symbol to `Owner#name`.
fn method_label(symbol: &str) -> String {
    let Some((owner, after)) = symbol.split_once('#') else {
        return symbol.to_string();
    };
    let name = after.split('(').next().unwrap_or(after);
    let owner = owner.rsplit('.').next().unwrap_or(owner);
    if owner.is_empty() {
        name.to_string()
    } else {
        format!("{owner}#{name}")
    }
}

/// Render a flow event array into PlantUML activity body.
/// Returns `None` if the flow has no usable steps.
pub fn activity_from_flow(events: &Value, symbol: &str) -> Option<String> {
    let rows = events.as_array()?;
    if rows.is_empty() {
        return None;
    }
    let esc = super::plantuml::escape;
    let mut out = String::new();
    out.push_str(&format!("' evidence: {}\n", symbol));
    // Tracks open `if` / `while` blocks so `END` closes the right construct.
    let mut stack: Vec<&str> = Vec::new();
    for row in rows {
        let kind = row["kind"].as_str().unwrap_or("");
        let text = row["text"].as_str().unwrap_or("");
        let condition = row["condition"].as_str().unwrap_or("");
        match kind {
            "IF" => {
                out.push_str(&format!("if ({}) then (да)\n", esc(condition)));
                stack.push("if");
            }
            "ELSE" | "ELSEIF" => {
                out.push_str("else (нет)\n");
            }
            "LOOP" => {
                let label = if condition.is_empty() {
                    "loop"
                } else {
                    condition
                };
                out.push_str(&format!("while ({}) is (да)\n", esc(label)));
                stack.push("loop");
            }
            "END" => match stack.pop() {
                Some("loop") => out.push_str("endwhile\n"),
                _ => out.push_str("endif\n"),
            },
            "CALL" | "CONSTRUCT" => {
                let target = row["target"].as_str().unwrap_or("");
                if !target.is_empty() {
                    out.push_str(&format!(":{};\n", esc(&method_label(target))));
                }
            }
            "RETURN" => {
                if !text.is_empty() {
                    out.push_str(&format!(":{}\n", esc(text)));
                }
            }
            "THROW" => {
                let what = if text.is_empty() { "error" } else { text };
                out.push_str(&format!(":throw {};\n", esc(what)));
            }
            "BOUNDARY" => {
                let what = if text.is_empty() { "boundary" } else { text };
                out.push_str(&format!(
                    "note right\n  {} (не развёрнуто)\nend note\n",
                    esc(what)
                ));
            }
            // STATEMENT / LOCAL carry no retained descriptive text; the step is
            // skipped rather than rendered as an empty action. Source deepening
            // (Slice A2) supplies readable text for these from TRANSFORMED_SOURCE.
            "STATEMENT" | "LOCAL" => {}
            _ => {
                if !text.is_empty() {
                    out.push_str(&format!(":{};\n", esc(text)));
                }
            }
        }
    }
    while let Some(branch) = stack.pop() {
        out.push_str(if branch == "loop" {
            "endwhile\n"
        } else {
            "endif\n"
        });
    }
    out.push_str("stop\n");
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
    fn call_target_is_rendered_as_owner_method() {
        let events = json!([
            {"kind":"CALL","resolution":"COMPILER_EXACT","target":"method:class:ru.tins.service.PolicyChangeTaskService#restartPolicyChangeTask(Lru/x;)Lru/y;"},
            {"kind":"RETURN"}
        ]);
        let src = "method:class:ru.tins.controller.PolicyChangeController#restartPolicyChangeTask(Lru/x;)Lru/y;";
        let body = super::activity_from_flow(&events, src).unwrap();
        assert!(
            body.contains(":PolicyChangeTaskService#restartPolicyChangeTask;"),
            "{body}"
        );
        assert!(body.contains("' evidence: method:class:ru.tins.controller.PolicyChangeController#restartPolicyChangeTask(Lru/x;)Lru/y;"));
        assert!(body.ends_with("stop\n"));
    }

    #[test]
    fn if_else_loop_end_close_correct_constructs() {
        let events = json!([
            {"kind":"IF","condition":"request != null"},
            {"kind":"CALL","target":"method:class:ru.tins.svc.Handler#handle()V"},
            {"kind":"LOOP","condition":"for-each"},
            {"kind":"CALL","target":"method:class:ru.tins.svc.Repo#save()V"},
            {"kind":"END"},
            {"kind":"ELSE"},
            {"kind":"RETURN"}
        ]);
        let body = super::activity_from_flow(&events, "method:s").unwrap();
        assert!(body.contains("if (request != null) then (да)"));
        assert!(body.contains("else (нет)"));
        assert!(body.contains("while (for-each) is (да)"));
        // END after LOOP closes the loop; the final unclosed IF closes at end.
        let while_open = body.match_indices("while (for-each) is (да)").count();
        let endwhile = body.match_indices("endwhile").count();
        let endif = body.match_indices("endif").count();
        assert_eq!(while_open, 1);
        assert_eq!(endwhile, 1, "{body}");
        assert_eq!(endif, 1, "{body}");
    }

    #[test]
    fn empty_flow_returns_none() {
        let events = json!([]);
        assert!(super::activity_from_flow(&events, "s").is_none());
    }
}
