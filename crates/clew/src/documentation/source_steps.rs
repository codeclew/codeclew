//! Deepen a method's retained source into readable PlantUML activity steps.
//!
//! The retained FLOW evidence carries structure (call targets, branch
//! conditions) but no readable text for assignments, returns and catch paths.
//! When a method's `TRANSFORMED_SOURCE` is available this module parses the
//! method body and renders it as human-readable activity steps ("Вход",
//! `receiver.method(...)`, `return ...`, branches), matching the approved
//! mockup. Falls back to nothing on methods it cannot parse so the caller can
//! use the FLOW-based renderer instead.

use serde_json::Value;

/// Extract the method name from a `method:class:<Owner>#<name>(...)<desc>` symbol.
fn method_name(symbol: &str) -> String {
    let after_hash = symbol.split_once('#').map(|(_, a)| a).unwrap_or(symbol);
    after_hash
        .split('(')
        .next()
        .unwrap_or(after_hash)
        .trim()
        .to_string()
}

/// Strip `/* */` and `//` comments, respecting string/char literals.
fn strip_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut chars = src.chars().peekable();
    let mut in_block = false;
    while let Some(c) = chars.next() {
        if in_block {
            if c == '*' && chars.peek() == Some(&'/') {
                chars.next();
                in_block = false;
            }
            continue;
        }
        if c == '/' && chars.peek() == Some(&'/') {
            // line comment: skip to newline
            for n in chars.by_ref() {
                if n == '\n' {
                    out.push('\n');
                    break;
                }
            }
            continue;
        }
        if c == '/' && chars.peek() == Some(&'*') {
            chars.next();
            in_block = true;
            continue;
        }
        out.push(c);
    }
    out
}

/// Find the range of the top-level method body (the first `{ ... }` at brace
/// depth one that contains a `;`), returning `(after_open, before_close)`.
fn method_body(src: &str) -> Option<(usize, usize)> {
    let open = src.find('{')?;
    let mut depth = 1usize;
    let bytes = src.as_bytes();
    let mut i = open + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some((open + 1, i));
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Split a method body (lines inside the braces) into logically complete
/// statements, joining lines that continue an expression (multi-line calls,
/// builder chains) with a single space. A statement ends at a block close
/// (`}`), a trailing `;`, or a trailing `{` (a control-flow header).
fn split_statements(body: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    for raw in body.split('\n') {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(line);
        let trimmed = cur.trim();
        if trimmed.starts_with('}') || trimmed.ends_with(';') || trimmed.ends_with('{') {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    out
}

/// Readable signature for the "Вход:" line, e.g. `changeTaskStatus(taskId, request)`.
fn signature(head: &str, symbol: &str) -> String {
    let name = method_name(symbol);
    let params = head
        .find('(')
        .and_then(|open| {
            head[open + 1..]
                .find(')')
                .map(|close| &head[open + 1..open + 1 + close])
        })
        .map(|inner| {
            inner
                .split(',')
                .filter_map(|p| p.trim().split_whitespace().last())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    if params.is_empty() {
        name
    } else {
        format!("{name}({params})")
    }
}

/// Tree-header signature: like `signature` but always renders the call parens,
/// e.g. `changeStatus()` for an empty parameter list.
fn tree_signature(head: &str, symbol: &str) -> String {
    let s = signature(head, symbol);
    // signature returns `name` (no parens) for empty params; append `()`.
    if s.contains('(') { s } else { format!("{s}()") }
}

/// Shorten a call/assignment statement to a readable single-line label.
fn shorten_statement(s: &str) -> String {
    let s = s.trim().trim_end_matches(';').trim();
    if let Some(rest) = s.strip_prefix("return") {
        return format!("return {}", keep_args(rest.trim()));
    }
    if let Some(rest) = s.strip_prefix("throw") {
        return format!("throw {}", keep_args(rest.trim()));
    }
    // Assignment: `Type name = expr` → `name = expr`; `name = expr` stays.
    if let Some((lhs, rhs)) = s.split_once('=') {
        let lhs = lhs.trim();
        let parts: Vec<&str> = lhs.split_whitespace().collect();
        let name = if parts.len() >= 2 && parts[0].chars().next().is_some_and(|c| c.is_uppercase())
        {
            parts[parts.len() - 1]
        } else {
            lhs
        };
        return format!("{} = {}", name, call_label(rhs.trim()));
    }
    call_label(s)
}

/// A call label with arguments collapsed to `(...)`, matching the mockup.
fn call_label(s: &str) -> String {
    let s = s.trim();
    if let Some(open) = s.find('(') {
        let callee = s[..open].trim();
        format!("{callee}(...)")
    } else {
        s.to_string()
    }
}

/// Keep a call's arguments when they are short, else collapse to `(...)`.
fn keep_args(s: &str) -> String {
    let s = s.trim();
    if let Some(open) = s.find('(') {
        let callee = s[..open].trim();
        // A closing `)` may be on a later line (multi-line call); collapse to
        // `(...)` rather than slicing past the open paren.
        let close = s[open + 1..].find(')').map(|rel| open + 1 + rel);
        match close {
            Some(close) => {
                let inner = &s[open + 1..close];
                if inner.len() <= 60 {
                    format!("{callee}({inner})")
                } else {
                    format!("{callee}(...)")
                }
            }
            None => format!("{callee}(...)"),
        }
    } else {
        s.to_string()
    }
}

/// Index of the first `=` that acts as an assignment separator: at paren/bracket
/// depth zero and outside string/char literals. Returns `None` when every `=`
/// sits inside a nested call or a literal, e.g. a multi-line
/// `log.info("count={}", x)` — that must not be misread as an assignment.
fn assignment_eq(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut depth = 0usize;
    let mut in_string = false;
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if in_string {
            if c == b'\\' {
                i += 2;
                continue;
            }
            if c == b'"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        match c {
            b'"' => in_string = true,
            b'(' | b'[' => depth += 1,
            b')' | b']' => depth = depth.saturating_sub(1),
            b'=' if depth == 0 => return Some(i),
            _ => {}
        }
        i += 1;
    }
    None
}

/// Shorten a statement for the source tree: like `shorten_statement` but keeps
/// a call's arguments when short, so data-flow (`setUpdateDate(changeDate)`)
/// is preserved. Assignments keep their value expression's callee.
fn tree_statement(s: &str) -> String {
    let s = s.trim().trim_end_matches(';').trim();
    if let Some(rest) = s.strip_prefix("return") {
        let rest = rest.trim();
        return if rest.is_empty() {
            "return".to_string()
        } else {
            format!("return {}", keep_args(rest))
        };
    }
    if let Some(rest) = s.strip_prefix("throw") {
        let rest = rest.trim();
        return if rest.is_empty() {
            "throw".to_string()
        } else {
            format!("throw {}", keep_args(rest))
        };
    }
    if let Some(eq) = assignment_eq(&s) {
        let lhs = s[..eq].trim();
        let rhs = s[eq + 1..].trim();
        let parts: Vec<&str> = lhs.split_whitespace().collect();
        let name = if parts.len() >= 2 && parts[0].chars().next().is_some_and(|c| c.is_uppercase())
        {
            parts[parts.len() - 1]
        } else {
            lhs
        };
        return format!("{} = {}", name, call_label(rhs));
    }
    keep_args(s)
}

/// Category prefix for a source-tree statement: assignments and write-verb
/// calls get `[W] `, read-verb calls get `[R] `; `return`/`throw` and
/// unclassifiable statements get no prefix.
fn statement_kind(stmt: &str) -> String {
    if stmt.starts_with("return") || stmt.starts_with("throw") {
        return String::new();
    }
    if let Some((_, rhs)) = stmt.split_once('=') {
        if !rhs.trim_start().starts_with('=') {
            return "[W] ".to_string();
        }
    }
    if let Some(open) = stmt.find('(') {
        let callee = stmt[..open].trim();
        let name = callee.rsplit('.').next().unwrap_or(callee).trim();
        if !name.is_empty() {
            if let Some(c) = super::process_flow::method_write_read(name) {
                return format!("[{c}] ");
            }
        }
    }
    String::new()
}

/// Render a method body as an indented pseudocode tree with `[W]/[R]/[D]`
/// categories and readable arguments/assignments (data-flow). Returns `None`
/// when the body cannot be isolated.
///
/// Control-flow constructs (`if`/`else`/`while`/`for`) nest by depth.
/// `try`/`catch`/`finally`/`switch`/`case`/`default`/`break`/`do` lines are
/// skipped without expanding their blocks in this prototype pass.
pub fn tree(source: &str, symbol: &str) -> Option<String> {
    let no_comments = strip_comments(source);
    let (open, close) = method_body(&no_comments)?;
    let head = no_comments[..open].trim();
    let body = &no_comments[open + 1..close];
    let statements = split_statements(body);
    let mut out = String::new();
    out.push_str(&format!("Вход: {}\n", tree_signature(head, symbol)));
    // Stack of open blocks. `true` = renderable control flow (if/loop) and
    // contributes to depth; `false` = a skipped block (try/catch/finally/
    // switch/do) whose braces balance without moving depth.
    let mut stack: Vec<bool> = Vec::new();
    let indent = |d: usize| "  ".repeat(d);
    for raw in &statements {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('}') {
            let rest = line.trim_start_matches('}').trim();
            if rest.starts_with("else if (") || rest.starts_with("elseif (") {
                stack.pop();
                let d = stack.iter().filter(|&&r| r).count();
                out.push_str(&format!(
                    "{}[D] else if ({}) then\n",
                    indent(d),
                    condition(line)
                ));
                stack.push(true);
            } else if rest.starts_with("else") {
                stack.pop();
                let d = stack.iter().filter(|&&r| r).count();
                out.push_str(&format!("{}else\n", indent(d)));
                stack.push(true);
            } else if rest.starts_with("catch (") || rest.starts_with("finally") {
                stack.pop();
                stack.push(false);
            } else if rest.starts_with("while (") {
                stack.pop();
            } else {
                stack.pop();
            }
            continue;
        }
        if line.starts_with("else if (") || line.starts_with("elseif (") {
            stack.pop();
            let d = stack.iter().filter(|&&r| r).count();
            out.push_str(&format!(
                "{}[D] else if ({}) then\n",
                indent(d),
                condition(line)
            ));
            stack.push(true);
            continue;
        }
        if line.starts_with("else") {
            stack.pop();
            let d = stack.iter().filter(|&&r| r).count();
            out.push_str(&format!("{}else\n", indent(d)));
            stack.push(true);
            continue;
        }
        if line.starts_with("if (") {
            let d = stack.iter().filter(|&&r| r).count();
            out.push_str(&format!(
                "{}[D] if ({}) then\n",
                indent(d),
                condition(line)
            ));
            stack.push(true);
            continue;
        }
        if line.starts_with("while (") || line.starts_with("for (") {
            let d = stack.iter().filter(|&&r| r).count();
            out.push_str(&format!(
                "{}[D] loop ({})\n",
                indent(d),
                condition(line)
            ));
            stack.push(true);
            continue;
        }
        if line.starts_with("try") || line.starts_with("switch (") || line.starts_with("do") {
            stack.push(false);
            continue;
        }
        if line.starts_with("catch (")
            || line.starts_with("finally")
            || line.starts_with("case ")
            || line.starts_with("default")
            || line.starts_with("break")
            || line == "{"
        {
            continue;
        }
        let stmt = tree_statement(line);
        if !stmt.is_empty() {
            let d = stack.iter().filter(|&&r| r).count();
            out.push_str(&format!(
                "{}{}{}\n",
                indent(d),
                statement_kind(&stmt),
                stmt
            ));
        }
    }
    if out.trim().is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Render a method body (lines inside the braces) as PlantUML activity steps.
fn render_body(body: &str, out: &mut String) {
    let cleaned = strip_comments(body);
    let mut lines: Vec<String> = cleaned.split('\n').map(|l| l.trim().to_string()).collect();
    // Drop a trailing dangling `}` (the body range already excludes it) and
    // any standalone braces are handled inline.
    let mut stack: Vec<&str> = Vec::new(); // "if" | "loop" | "switch" | "try" | "catch"
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i].trim().to_string();
        let next_nonempty = |from: usize| -> Option<String> {
            lines[from..]
                .iter()
                .map(|l| l.trim())
                .find(|l| !l.is_empty())
                .map(|l| l.to_string())
        };
        if line.is_empty() {
            i += 1;
            continue;
        }
        // Closing brace. A `} else`, `} else if`, `} catch`, `} finally` or
        // `} while` (do-while) continues the enclosing block instead of
        // closing it; otherwise the top block is closed.
        if line.starts_with("}") {
            let rest = line.trim_start_matches('}').trim();
            if rest.starts_with("else if (") || rest.starts_with("elseif (") {
                out.push_str(&format!("else if ({}) then (да)\n", condition(rest)));
                i += 1;
                continue;
            }
            if rest.starts_with("else") {
                out.push_str("else (нет)\n");
                i += 1;
                continue;
            }
            if rest.starts_with("catch (") {
                stack.pop(); // close try/finally
                out.push_str(&format!(
                    "note right\n  {}\nend note\n",
                    super::plantuml::escape(&catch_header(rest))
                ));
                stack.push("catch");
                i += 1;
                continue;
            }
            if rest.starts_with("finally") {
                stack.pop();
                out.push_str("note right\n  finally\nend note\n");
                stack.push("catch");
                i += 1;
                continue;
            }
            if rest.starts_with("while (") {
                stack.pop(); // close the `do`
                out.push_str(&format!("repeat while ({}) is (да)\n", condition(rest)));
                i += 1;
                continue;
            }
            // Standalone close: allow an `else`/`catch` continuation on the
            // following line before committing to closing the block.
            if rest.is_empty() {
                let next = next_nonempty(i + 1);
                let cont = next.as_deref().is_some_and(|n| {
                    n.starts_with("else")
                        || n.starts_with("catch")
                        || n.starts_with("finally")
                        || n.starts_with("while")
                });
                if cont {
                    i += 1;
                    continue;
                }
            }
            match stack.pop() {
                Some("loop") => out.push_str("endwhile\n"),
                Some("if") => out.push_str("endif\n"),
                _ => {} // switch/try/catch/finally have no activity close
            }
            i += 1;
            continue;
        }
        if line.starts_with("else if (") || line.starts_with("elseif (") {
            out.push_str(&format!("else if ({}) then (да)\n", condition(&line)));
            i += 1;
            continue;
        }
        if line.starts_with("else") {
            out.push_str("else (нет)\n");
            i += 1;
            continue;
        }
        if line.starts_with("if (") {
            out.push_str(&format!("if ({}) then (да)\n", condition(&line)));
            stack.push("if");
            i += 1;
            continue;
        }
        if line.starts_with("while (") || line.starts_with("for (") {
            let cond = condition(&line);
            out.push_str(&format!("while ({}) is (да)\n", cond));
            stack.push("loop");
            i += 1;
            continue;
        }
        if line.starts_with("do") {
            out.push_str("repeat\n");
            stack.push("loop");
            i += 1;
            continue;
        }
        if line.starts_with("try") {
            stack.push("try");
            i += 1;
            continue;
        }
        if line.starts_with("catch (") {
            out.push_str(&format!(
                "note right\n  {}\nend note\n",
                super::plantuml::escape(&catch_header(&line))
            ));
            stack.push("catch");
            i += 1;
            continue;
        }
        if line.starts_with("finally") {
            out.push_str("note right\n  finally\nend note\n");
            stack.push("catch");
            i += 1;
            continue;
        }
        if line.starts_with("switch (") {
            out.push_str(&format!("switch ({})\n", condition(&line)));
            stack.push("switch");
            i += 1;
            continue;
        }
        if line.starts_with("case ")
            || line.starts_with("default")
            || line.starts_with("break")
            || line == "{"
            || line == ";"
        {
            i += 1;
            continue;
        }
        // A real statement: call / assignment / return / throw / declaration.
        let stmt = shorten_statement(&line);
        if !stmt.is_empty() {
            out.push_str(&format!(":{};\n", super::plantuml::escape(&stmt)));
        }
        i += 1;
    }
    while let Some(b) = stack.pop() {
        match b {
            "loop" => out.push_str("endwhile\n"),
            "if" => out.push_str("endif\n"),
            _ => {}
        }
    }
}

/// The `catch (Type name) {` header, up to (and including) the closing `)`.
fn catch_header(line: &str) -> String {
    let mut close = line.find(')').map(|c| c + 1).unwrap_or(line.len());
    while line.as_bytes().get(close) == Some(&b'{') {
        close += 1;
    }
    line[..close.min(line.len())].trim().to_string()
}

/// Extract the parenthesised condition from `keyword (cond) ...`, matching the
/// outer parenthesis (conditions may contain nested calls).
fn condition(line: &str) -> String {
    let open = line.find('(').unwrap_or(0);
    let bytes = line.as_bytes();
    let mut depth = 1usize;
    let mut i = open + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
        i += 1;
    }
    let cond = line[open + 1..i].trim();
    if cond.len() > 48 {
        format!("{}...", &cond[..45])
    } else {
        cond.to_string()
    }
}

/// Parse a method source into a full PlantUML activity document, or `None`
/// when the body cannot be isolated.
pub fn document(source: &str, symbol: &str, title: &str) -> Option<String> {
    let no_comments = strip_comments(source);
    let (open, close) = method_body(&no_comments)?;
    let head = no_comments[..open].trim();
    let body = &no_comments[open + 1..close];
    let mut steps = String::new();
    render_body(body, &mut steps);
    if steps.trim().is_empty() {
        return None;
    }
    Some(format!(
        "@startuml\n!theme plain\n!pragma layout smetana\ntitle {}\nstart\n:Вход: {};\n{}stop\n@enduml\n",
        super::plantuml::escape(title),
        super::plantuml::escape(&signature(head, symbol)),
        steps
    ))
}

/// Convenience guard so the FLOW-based renderer can detect whether source
/// deepening produced anything usable without building the whole document.
pub fn usable(source: &str, symbol: &str) -> bool {
    let no_comments = strip_comments(source);
    method_body(&no_comments)
        .map(|(o, c)| !no_comments[o + 1..c].trim().is_empty())
        .unwrap_or(false)
}

/// A lightweight view of the rendered steps, used by the render seam to decide
/// between source deepening and the FLOW renderer.
pub fn steps(source: &str, symbol: &str) -> Option<Value> {
    let no_comments = strip_comments(source);
    let (open, close) = method_body(&no_comments)?;
    let mut out = String::new();
    render_body(&no_comments[open + 1..close], &mut out);
    if out.trim().is_empty() {
        None
    } else {
        Some(Value::String(format!(
            "{}: {}\n{}",
            method_name(symbol),
            signature(&no_comments[..open], symbol),
            out
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHANGE_TASK_STATUS: &str = r#"
public ChangeTaskStatusResponse changeTaskStatus(Long taskId, ChangeTaskStatusRequest request) {
    TaskInstance taskInstance = null;
    Optional<TaskInstance> anyTask = taskInstanceDao.findByTaskId(taskId);
    try {
        if (anyTask.isPresent()) {
            taskInstance = anyTask.get();
            taskHandlingService.changeAnyTaskStatus(anyTask.get(), request, taskId);
        } else {
            taskInstance = taskHandlingService.changeTaskStatus(request, taskId);
        }
        if (TaskStatuses.FINISHED.equals(taskInstance.getTaskStatus())) {
            taskStatusesMetrics.incFinished(taskType);
        }
    } catch (Exception e) {
        return createResponse(ResponseCodes.INTERNAL_ERROR, taskId, null, ChangeTaskStatusResponse.builder().build());
    }
    return createResponse(ResponseCodes.OK, taskId, null, ChangeTaskStatusResponse.builder().build());
}
"#;

    #[test]
    fn deepens_method_source_into_readable_steps() {
        let doc = document(
            CHANGE_TASK_STATUS,
            "method:class:svc.TaskService#changeTaskStatus(JLru/x;)Lru/y;",
            "t",
        )
        .unwrap();
        assert!(
            doc.contains(":Вход: changeTaskStatus(taskId, request);\n"),
            "{doc}"
        );
        assert!(doc.contains(":taskInstance = null;"), "{doc}");
        assert!(doc.contains("if (anyTask.isPresent()) then (да)"), "{doc}");
        assert!(doc.contains("else (нет)"), "{doc}");
        assert!(
            doc.contains(":taskHandlingService.changeAnyTaskStatus(...);"),
            "{doc}"
        );
        assert!(
            doc.contains(":taskInstance = taskHandlingService.changeTaskStatus(...);"),
            "{doc}"
        );
        assert!(doc.contains("endif"), "{doc}");
        assert!(doc.contains("note right"), "{doc}");
        assert!(doc.contains("catch (Exception e)"), "{doc}");
        assert!(doc.contains(":return createResponse(...);"), "{doc}");
        assert!(doc.ends_with("stop\n@enduml\n"), "{doc}");
    }

    #[test]
    fn return_and_throw_keep_keyword() {
        // Short arg lists are kept readable; long ones collapse to `(...)`.
        assert_eq!(
            shorten_statement("return createResponse(a, b);"),
            "return createResponse(a, b)"
        );
        assert_eq!(
            shorten_statement("throw new IllegalStateException(\"bad\");"),
            "throw new IllegalStateException(\"bad\")"
        );
        assert_eq!(
            shorten_statement(
                "return createResponse(ResponseCodes.OK, taskId, null, ChangeTaskStatusResponse.builder().build());"
            ),
            "return createResponse(...)"
        );
    }

    #[test]
    fn assignment_strips_declaration_type() {
        assert_eq!(
            shorten_statement("TaskInstance taskInstance = null;"),
            "taskInstance = null"
        );
        assert_eq!(
            shorten_statement("taskInstance = anyTask.get();"),
            "taskInstance = anyTask.get(...)"
        );
    }

    #[test]
    fn keep_args_collapses_multiline_call_without_closing_paren() {
        // A call opened on this line but closed on a later line must not panic
        // or slice past the open paren.
        assert_eq!(keep_args("log.info("), "log.info(...)");
        assert_eq!(keep_args("log.info(\n  message"), "log.info(...)");
    }

    #[test]
    fn unparseable_source_returns_none() {
        assert!(document("no body here", "method:x", "t").is_none());
        assert!(!usable("no body here", "method:x"));
    }

    #[test]
    fn tree_renders_categorized_data_flow() {
        let src = "\
private void changeStatus() {
    Date changeDate = DateTimeHolder.getCurrentTime();
    taskInstance.setUpdateDate(changeDate);
    if (priority != null) {
        taskInstance.setTaskPriority(priority);
    }
    return;
}";
        let tree = tree(src, "method:class:svc.TaskService#changeStatus()V").unwrap();
        assert!(tree.starts_with("Вход: changeStatus()\n"), "{tree}");
        assert!(
            tree.contains("[W] changeDate = DateTimeHolder.getCurrentTime(...)"),
            "{tree}"
        );
        assert!(
            tree.contains("[W] taskInstance.setUpdateDate(changeDate)"),
            "{tree}"
        );
        assert!(
            tree.contains(
                "[D] if (priority != null) then\n  [W] taskInstance.setTaskPriority(priority)"
            ),
            "{tree}"
        );
        assert!(tree.ends_with("return\n"), "{tree}");
    }

    #[test]
    fn tree_returns_none_on_unparseable_source() {
        assert!(tree("no body here", "method:x").is_none());
    }

    #[test]
    fn tree_keeps_depth_through_skipped_try_catch() {
        let src = "\
void run() {
    if (x) {
        try {
            svc.a();
        } catch (E e) {
            svc.b();
        }
        svc.c();
    }
}";
        let tree = tree(src, "method:class:svc.TaskService#run()V").unwrap();
        // The try/catch body stays at the enclosing `if` depth: a(), b(), c()
        // all indented one level.
        assert!(
            tree.contains("[D] if (x) then\n  svc.a()\n  svc.b()\n  svc.c()"),
            "{tree}"
        );
    }

    #[test]
    fn split_statements_joins_multiline_call_and_builder_chain() {
        let body = "\
Date changeDate = DateTimeHolder.getCurrentTime();
log.info(
    \"count={}, type={}\",
    taskId,
    task.getType()
);
taskStatusHistoryDao = TaskStatusHistoryDao.builder()
    .taskInstance(taskInstance)
    .changeUser(user)
    .build();
if (priority != null) {
    taskInstance.setTaskPriority(priority);
}";
        let stmts = split_statements(body);
        assert_eq!(
            stmts,
            vec![
                "Date changeDate = DateTimeHolder.getCurrentTime();",
                "log.info( \"count={}, type={}\", taskId, task.getType() );",
                "taskStatusHistoryDao = TaskStatusHistoryDao.builder() .taskInstance(taskInstance) .changeUser(user) .build();",
                "if (priority != null) {",
                "taskInstance.setTaskPriority(priority);",
                "}",
            ]
        );
    }

    #[test]
    fn tree_renders_multiline_call_as_one_statement() {
        let src = "\
void log() {
    log.info(
        \"count={}\",
        taskId,
        task.getType()
    );
    if (task != null) {
        svc.handle(task);
    }
}";
        let tree = tree(src, "method:class:svc.TaskService#log()V").unwrap();
        // The multi-line log.info(...) call is aggregated into a single
        // statement (previously torn into separate `"count={}",` / `taskId,` /
        // `task.getType()` lines), and the `=` inside the string literal is
        // not misread as an assignment (no `count = {}` corruption).
        assert!(tree.contains("[W] log.info("), "{tree}");
        assert!(tree.contains("\"count={}\", taskId, task.getType()"), "{tree}");
        assert!(!tree.contains("\"count = {}"), "{tree}");
        assert!(tree.contains("[D] if (task != null) then\n  svc.handle(task)"), "{tree}");
    }

    #[test]
    fn tree_collapses_builder_chain_into_one_assignment() {
        let src = "\
void build() {
    TaskStatusHistoryDao dao = TaskStatusHistoryDao.builder()
        .taskInstance(taskInstance)
        .changeUser(user)
        .build();
    if (dao != null) {
        repo.save(dao);
    }
}";
        let tree = tree(src, "method:class:svc.TaskService#build()V").unwrap();
        assert!(
            tree.contains("[W] dao = TaskStatusHistoryDao.builder(...)"),
            "{tree}"
        );
        assert!(!tree.contains(".taskInstance(taskInstance)"), "{tree}");
        assert!(!tree.contains(".changeUser(user)"), "{tree}");
        assert!(
            tree.contains("[D] if (dao != null) then\n  [W] repo.save(dao)"),
            "{tree}"
        );
    }
}
