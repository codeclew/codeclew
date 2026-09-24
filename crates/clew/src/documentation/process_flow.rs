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

/// True when a `method:class:<package>...` target belongs to the runtime/JDK
/// rather than the service domain. Framework plumbing (collections, Optional,
/// Spring, JDBC) adds noise to a process diagram and is filtered out.
fn is_framework_symbol(symbol: &str) -> bool {
    let package = symbol.strip_prefix("method:class:").unwrap_or(symbol);
    const PREFIXES: &[&str] = &[
        "java.",
        "javax.",
        "sun.",
        "com.sun.",
        "kotlin.",
        "org.springframework",
        "org.apache.",
        "org.slf4j",
        "lombok.",
    ];
    PREFIXES.iter().any(|p| package.starts_with(p))
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
                if !target.is_empty() && !is_framework_symbol(target) {
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
        "@startuml\n!theme plain\n!pragma layout smetana\ntitle {}\nstart\n:Вход: {};\n{body}\n@enduml\n",
        super::plantuml::escape(title),
        super::plantuml::escape(&method_label(symbol))
    ))
}

/// Render a flow as an indented pseudocode tree (readable in a `<pre>` block)
/// instead of a diagram image. Root line carries the method, then each step is
/// indented by block depth; branches and loops open a level, `else` continues
/// the current block.
pub fn tree(events: &Value, symbol: &str) -> Option<String> {
    let rows = events.as_array()?;
    if rows.is_empty() {
        return None;
    }
    let mut out = String::new();
    out.push_str(&format!("Вход: {}\n", method_label(symbol)));
    let mut depth = 0usize;
    let indent = |d: usize| "  ".repeat(d);
    for row in rows {
        let kind = row["kind"].as_str().unwrap_or("");
        match kind {
            "IF" => {
                out.push_str(&format!("{}if ({}) then\n", indent(depth), condition(&row)));
                depth += 1;
            }
            "ELSE" | "ELSEIF" => {
                depth = depth.saturating_sub(1);
                out.push_str(&format!("{}else\n", indent(depth)));
                depth += 1;
            }
            "END" => depth = depth.saturating_sub(1),
            "LOOP" => {
                out.push_str(&format!("{}loop ({})\n", indent(depth), condition(&row)));
                depth += 1;
            }
            "CALL" | "CONSTRUCT" => {
                let target = row["target"].as_str().unwrap_or("");
                if !target.is_empty() && !is_framework_symbol(target) {
                    out.push_str(&format!("{}{}\n", indent(depth), method_label(target)));
                }
            }
            "RETURN" => out.push_str(&format!("{}return\n", indent(depth))),
            "THROW" => out.push_str(&format!("{}throw\n", indent(depth))),
            "BOUNDARY" => out.push_str(&format!("{}... (не развёрнуто)\n", indent(depth))),
            _ => {}
        }
    }
    Some(out)
}

/// Readable condition text from a flow row's `condition` field.
fn condition(row: &Value) -> String {
    let c = row["condition"].as_str().unwrap_or("");
    if c.len() > 72 {
        format!("{}...", &c[..69])
    } else {
        c.to_string()
    }
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
        assert!(super::tree(&events, "s").is_none());
    }

    #[test]
    fn tree_renders_indented_pseudocode_branches() {
        let events = json!([
            {"kind":"IF","condition":"anyTask.isPresent()"},
            {"kind":"CALL","target":"method:class:ru.tins.svc.Handler#handle()V"},
            {"kind":"ELSE"},
            {"kind":"CALL","target":"method:class:ru.tins.svc.Other#do()V"},
            {"kind":"END"},
            {"kind":"CALL","target":"method:class:ru.tins.svc.Repo#save()V"},
            {"kind":"RETURN"}
        ]);
        let tree = super::tree(
            &events,
            "method:class:ru.tins.svc.TaskService#changeStatus()V",
        )
        .unwrap();
        assert!(
            tree.starts_with("Вход: TaskService#changeStatus\n"),
            "{tree}"
        );
        assert!(
            tree.contains("if (anyTask.isPresent()) then\n  Handler#handle"),
            "{tree}"
        );
        assert!(tree.contains("else\n  Other#do"), "{tree}");
        assert!(tree.contains("Repo#save\nreturn"), "{tree}");
    }

    #[test]
    fn framework_calls_are_filtered_out() {
        let events = json!([
            {"kind":"CALL","target":"method:class:java.util.Optional#ofNullable(Ljava/lang/Object;)Ljava/util/Optional;"},
            {"kind":"CALL","target":"method:class:org.springframework.data.repository.CrudRepository#save(Ljava/lang/Object;)Ljava/lang/Object;"},
            {"kind":"CALL","target":"method:class:ru.tins.task.service.TaskService#save(Lru/Task;)V"},
            {"kind":"RETURN"}
        ]);
        let body = super::activity_from_flow(&events, "method:s").unwrap();
        assert!(!body.contains("Optional#ofNullable"), "{body}");
        assert!(!body.contains("CrudRepository#save"), "{body}");
        assert!(body.contains(":TaskService#save;"), "{body}");
    }

    #[test]
    fn document_emits_vhod_header() {
        let events = json!([
            {"kind":"CALL","target":"method:class:ru.tins.svc.TaskService#changeStatus()V"},
            {"kind":"RETURN"}
        ]);
        let doc = super::document(
            &events,
            "method:class:ru.tins.svc.TaskService#changeStatus()V",
            "t",
        )
        .unwrap();
        assert!(doc.contains(":Вход: TaskService#changeStatus;\n"), "{doc}");
    }
}
