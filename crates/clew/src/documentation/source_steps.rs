//! Deepen a method's retained source into readable PlantUML activity steps.
//!
//! The retained FLOW evidence carries structure (call targets, branch
//! conditions) but no readable text for assignments, returns and catch paths.
//! When a method's `TRANSFORMED_SOURCE` is available this module parses the
//! method body and renders it as human-readable activity steps ("Entry",
//! `receiver.method(...)`, `return ...`, branches), matching the approved
//! mockup. Falls back to nothing on methods it cannot parse so the caller can
//! use the FLOW-based renderer instead.

use serde_json::Value;

const MAX_SOURCE_BYTES: usize = 256 * 1024;
const MAX_BLOCK_DEPTH: usize = 128;
const MAX_STATEMENTS: usize = 10_000;

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

/// Mark source bytes that are code and bytes that belong to comments.
/// String, character, and triple-quoted literals are opaque to syntax scans.
fn lexical_masks(src: &str) -> (Vec<bool>, Vec<bool>) {
    let bytes = src.as_bytes();
    let mut code = vec![false; bytes.len()];
    let mut comment = vec![false; bytes.len()];
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'/') {
            let start = i;
            i += 2;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            comment[start..i].fill(true);
            continue;
        }
        if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'*') {
            let start = i;
            i += 2;
            while i < bytes.len() {
                if bytes[i] == b'*' && bytes.get(i + 1) == Some(&b'/') {
                    i += 2;
                    break;
                }
                i += 1;
            }
            comment[start..i].fill(true);
            continue;
        }
        if bytes[i] == b'"' || bytes[i] == b'\'' {
            let quote = bytes[i];
            let text_block = quote == b'"' && bytes.get(i..i + 3) == Some(b"\"\"\"");
            i += if text_block { 3 } else { 1 };
            while i < bytes.len() {
                if bytes[i] == b'\\' {
                    i = (i + 2).min(bytes.len());
                    continue;
                }
                if text_block {
                    if bytes.get(i..i + 3) == Some(b"\"\"\"") {
                        i += 3;
                        break;
                    }
                } else if bytes[i] == quote {
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }
        code[i] = true;
        i += 1;
    }
    (code, comment)
}

/// Strip comments while preserving newlines and literal contents.
fn strip_comments(src: &str) -> String {
    let (_, comment) = lexical_masks(src);
    let mut out = src.as_bytes().to_vec();
    for (i, is_comment) in comment.into_iter().enumerate() {
        if is_comment && out[i] != b'\n' {
            out[i] = b' ';
        }
    }
    String::from_utf8(out).expect("comment replacement preserves UTF-8")
}

/// Find the unique method body named by the retained FLOW symbol.
fn method_body(src: &str, symbol: &str) -> Option<(usize, usize)> {
    let (code, _) = lexical_masks(src);
    let bytes = src.as_bytes();
    let name = method_name(symbol);
    let needle = name.as_bytes();
    if needle.is_empty() || needle.len() > bytes.len() {
        return None;
    }

    let mut found = None;
    for start in 0..=bytes.len() - needle.len() {
        let end_name = start + needle.len();
        if &bytes[start..end_name] != needle || !code[start..end_name].iter().all(|b| *b) {
            continue;
        }
        let is_identifier = |b: Option<&u8>| {
            b.is_some_and(|b| b.is_ascii_alphanumeric() || matches!(*b, b'_' | b'$') || *b >= 0x80)
        };
        if is_identifier(start.checked_sub(1).and_then(|i| bytes.get(i)))
            || is_identifier(bytes.get(end_name))
        {
            continue;
        }

        let mut open_paren = end_name;
        while bytes.get(open_paren).is_some_and(u8::is_ascii_whitespace) {
            open_paren += 1;
        }
        if bytes.get(open_paren) != Some(&b'(') || !code[open_paren] {
            continue;
        }
        let Some(close_paren) = matching_delimiter(src, &code, open_paren, b'(', b')') else {
            continue;
        };
        let mut body_open = close_paren + 1;
        while body_open < bytes.len() {
            if !code[body_open] || bytes[body_open].is_ascii_whitespace() {
                body_open += 1;
                continue;
            }
            match bytes[body_open] {
                b'{' => break,
                b';' | b'}' => {
                    body_open = bytes.len();
                    break;
                }
                _ => body_open += 1,
            }
        }
        if body_open >= bytes.len() || bytes[body_open] != b'{' {
            continue;
        }
        if found.is_some() {
            return None;
        }
        found = Some(body_open);
    }

    let open = found?;
    let mut depth = 1usize;
    let mut i = open + 1;
    while i < bytes.len() {
        if !code[i] {
            i += 1;
            continue;
        }
        match bytes[i] {
            b'{' => {
                depth += 1;
                if depth > MAX_BLOCK_DEPTH {
                    return None;
                }
            }
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some((open, i));
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn matching_delimiter(
    src: &str,
    code: &[bool],
    open: usize,
    opening: u8,
    closing: u8,
) -> Option<usize> {
    let bytes = src.as_bytes();
    let mut depth = 0usize;
    for i in open..bytes.len() {
        if !code[i] {
            continue;
        }
        match bytes[i] {
            b if b == opening => depth += 1,
            b if b == closing => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

fn first_code_byte(source: &str, needle: u8) -> Option<usize> {
    let (code, _) = lexical_masks(source);
    source
        .as_bytes()
        .iter()
        .enumerate()
        .find_map(|(i, byte)| (code[i] && *byte == needle).then_some(i))
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        text.to_string()
    } else {
        format!("{}...", text.chars().take(max_chars).collect::<String>())
    }
}

fn starts_control_header(header: &str) -> bool {
    [
        "if", "else", "while", "for", "try", "catch", "finally", "switch",
    ]
    .iter()
    .any(|keyword| {
        header.strip_prefix(keyword).is_some_and(|rest| {
            rest.is_empty()
                || rest.chars().next().is_some_and(char::is_whitespace)
                || rest.starts_with('(')
                || rest.starts_with('{')
        })
    })
}

fn normalize_control_header(header: &str) -> String {
    let header = compact_whitespace(header);
    let (keyword, rest) = if let Some(rest) = header.strip_prefix("else if") {
        ("else if", rest)
    } else if let Some(rest) = header.strip_prefix("if") {
        ("if", rest)
    } else if let Some(rest) = header.strip_prefix("while") {
        ("while", rest)
    } else if let Some(rest) = header.strip_prefix("for") {
        ("for", rest)
    } else if let Some(rest) = header.strip_prefix("catch") {
        ("catch", rest)
    } else if let Some(rest) = header.strip_prefix("switch") {
        ("switch", rest)
    } else if let Some(rest) = header.strip_prefix("else") {
        ("else", rest)
    } else if let Some(rest) = header.strip_prefix("finally") {
        ("finally", rest)
    } else {
        ("try", header.strip_prefix("try").unwrap_or(""))
    };
    let rest = rest.trim_start();
    if rest.is_empty() {
        keyword.to_string()
    } else {
        format!("{keyword} {rest}")
    }
}

fn is_control_statement(statement: &str) -> bool {
    [
        "if", "else", "while", "for", "try", "catch", "finally", "switch", "do",
    ]
    .iter()
    .any(|keyword| {
        statement.strip_prefix(keyword).is_some_and(|rest| {
            rest.is_empty()
                || rest.chars().next().is_some_and(char::is_whitespace)
                || rest.starts_with('(')
                || rest.starts_with('{')
        })
    })
}

fn compact_whitespace(text: &str) -> String {
    let (code, _) = lexical_masks(text);
    let mut out = String::with_capacity(text.len());
    let mut pending_space = false;
    for (i, ch) in text.char_indices() {
        if code[i] && ch.is_whitespace() {
            pending_space = !out.is_empty();
            continue;
        }
        if pending_space {
            out.push(' ');
            pending_space = false;
        }
        out.push(ch);
    }
    out.trim().to_string()
}

/// Split a body into complete statements and supported control blocks.
/// Delimiters inside literals, comments, calls, and array accesses are ignored.
/// Unsupported brace expressions return `None`, allowing FLOW rendering to
/// provide a fallback without inventing source steps.
fn split_statements(body: &str) -> Option<Vec<String>> {
    if body.len() > MAX_SOURCE_BYTES {
        return None;
    }
    let cleaned = strip_comments(body);
    let body = cleaned.as_str();
    let (code, _) = lexical_masks(body);
    let bytes = body.as_bytes();
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut block_depth = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        if !code[i] {
            i += 1;
            continue;
        }
        match bytes[i] {
            b'(' => paren_depth += 1,
            b')' => {
                if paren_depth == 0 {
                    return None;
                }
                paren_depth -= 1;
            }
            b'[' => bracket_depth += 1,
            b']' => {
                if bracket_depth == 0 {
                    return None;
                }
                bracket_depth -= 1;
            }
            b'{' if paren_depth == 0 && bracket_depth == 0 => {
                let header = compact_whitespace(body[start..i].trim());
                if !starts_control_header(&header) || block_depth >= MAX_BLOCK_DEPTH {
                    return None;
                }
                let header = normalize_control_header(&header);
                out.push(format!("{header} {{"));
                block_depth += 1;
                start = i + 1;
            }
            b'}' if paren_depth == 0 && bracket_depth == 0 => {
                let pending = compact_whitespace(body[start..i].trim());
                if !pending.is_empty() {
                    out.push(pending);
                }
                if block_depth == 0 {
                    return None;
                }
                out.push("}".to_string());
                block_depth -= 1;
                start = i + 1;
            }
            b';' if paren_depth == 0 && bracket_depth == 0 => {
                let statement = compact_whitespace(body[start..=i].trim());
                if !statement.is_empty() {
                    out.push(statement);
                }
                start = i + 1;
            }
            _ => {}
        }
        if out.len() > MAX_STATEMENTS {
            return None;
        }
        i += 1;
    }
    if paren_depth != 0 || bracket_depth != 0 || block_depth != 0 {
        return None;
    }
    let pending = compact_whitespace(body[start..].trim());
    if !pending.is_empty() {
        out.push(pending);
    }
    if out.len() > MAX_STATEMENTS {
        return None;
    }
    if out.iter().any(|statement| {
        !statement.starts_with('}') && is_control_statement(statement) && !statement.ends_with('{')
    }) {
        return None;
    }
    // Attach block continuations to the close they continue. The renderers
    // already understand this form for else, catch, finally, and do-while.
    let mut joined = Vec::with_capacity(out.len());
    let mut i = 0;
    while i < out.len() {
        if out[i] == "}" && i + 1 < out.len() {
            let next = out[i + 1].trim_start();
            if ["else", "catch", "finally"]
                .iter()
                .any(|word| next.starts_with(word))
            {
                joined.push(format!("}} {next}"));
                i += 2;
                continue;
            }
        }
        joined.push(std::mem::take(&mut out[i]));
        i += 1;
    }
    Some(joined)
}

/// Readable signature for the "Entry:" line, e.g. `changeTaskStatus(taskId, request)`.
fn signature(head: &str, symbol: &str) -> String {
    let name = method_name(symbol);
    let params = method_parameters(head, &name)
        .map(|inner| {
            inner
                .split(',')
                .filter_map(|p| p.split_whitespace().last())
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

fn method_parameters<'a>(head: &'a str, name: &str) -> Option<&'a str> {
    let bytes = head.as_bytes();
    let needle = name.as_bytes();
    if needle.is_empty() || needle.len() > bytes.len() {
        return None;
    }
    let (code, _) = lexical_masks(head);
    let mut result = None;
    for start in 0..=bytes.len() - needle.len() {
        let end = start + needle.len();
        if &bytes[start..end] != needle || !code[start..end].iter().all(|b| *b) {
            continue;
        }
        let boundary = |byte: Option<&u8>| {
            byte.is_some_and(|b| {
                b.is_ascii_alphanumeric() || matches!(*b, b'_' | b'$') || *b >= 0x80
            })
        };
        if boundary(start.checked_sub(1).and_then(|i| bytes.get(i))) || boundary(bytes.get(end)) {
            continue;
        }
        let mut open = end;
        while bytes.get(open).is_some_and(u8::is_ascii_whitespace) {
            open += 1;
        }
        if bytes.get(open) != Some(&b'(') || !code[open] {
            continue;
        }
        let Some(close) = matching_delimiter(head, &code, open, b'(', b')') else {
            continue;
        };
        if result.is_some() {
            return None;
        }
        result = Some(&head[open + 1..close]);
    }
    result
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
    if let Some(eq) = assignment_eq(s) {
        let (lhs, rhs) = s.split_at(eq);
        let rhs = &rhs[1..];
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
    if let Some(open) = first_code_byte(s, b'(') {
        let callee = s[..open].trim();
        format!("{callee}(...)")
    } else {
        s.to_string()
    }
}

/// Keep a call's arguments when they are short, else collapse to `(...)`.
fn keep_args(s: &str) -> String {
    let s = s.trim();
    if let Some(open) = first_code_byte(s, b'(') {
        let callee = s[..open].trim();
        let (code, _) = lexical_masks(s);
        match matching_delimiter(s, &code, open, b'(', b')') {
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
    let (code, _) = lexical_masks(s);
    let bytes = s.as_bytes();
    let mut depth = 0usize;
    let mut i = 0;
    while i < bytes.len() {
        if !code[i] {
            i += 1;
            continue;
        }
        let c = bytes[i];
        match c {
            b'(' | b'[' => depth += 1,
            b')' | b']' => depth = depth.saturating_sub(1),
            b'=' if depth == 0 => {
                let previous = i.checked_sub(1).and_then(|p| bytes.get(p));
                let next = bytes.get(i + 1);
                if previous.is_some_and(|b| matches!(*b, b'=' | b'!' | b'<' | b'>'))
                    || next == Some(&b'=')
                {
                    i += 1;
                    continue;
                }
                return Some(i);
            }
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
    if let Some(eq) = assignment_eq(s) {
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
    if assignment_eq(stmt).is_some() {
        return "[W] ".to_string();
    }
    if let Some(open) = first_code_byte(stmt, b'(') {
        let callee = stmt[..open].trim();
        let name = callee.rsplit('.').next().unwrap_or(callee).trim();
        if !name.is_empty()
            && let Some(c) = super::process_flow::method_write_read(name)
        {
            return format!("[{c}] ");
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
    if source.len() > MAX_SOURCE_BYTES {
        return None;
    }
    let no_comments = strip_comments(source);
    let (open, close) = method_body(&no_comments, symbol)?;
    let head = no_comments[..open].trim();
    let body = &no_comments[open + 1..close];
    let statements = split_statements(body)?;
    if statements.is_empty() {
        return None;
    }
    let mut out = String::new();
    out.push_str(&format!("Entry: {}\n", tree_signature(head, symbol)));
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
            out.push_str(&format!("{}[D] if ({}) then\n", indent(d), condition(line)));
            stack.push(true);
            continue;
        }
        if line.starts_with("while (") || line.starts_with("for (") {
            let d = stack.iter().filter(|&&r| r).count();
            out.push_str(&format!("{}[D] loop ({})\n", indent(d), condition(line)));
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
            out.push_str(&format!("{}{}{}\n", indent(d), statement_kind(&stmt), stmt));
        }
    }
    if out.trim().is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Render a method body (lines inside the braces) as PlantUML activity steps.
fn render_body(body: &str, out: &mut String) -> bool {
    let cleaned = strip_comments(body);
    let Some(lines) = split_statements(&cleaned) else {
        return false;
    };
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
                out.push_str(&format!(
                    "else if ({}) then (yes)\n",
                    super::plantuml::escape(&condition(rest))
                ));
                i += 1;
                continue;
            }
            if rest.starts_with("else") {
                out.push_str("else (no)\n");
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
                out.push_str(&format!(
                    "repeat while ({}) is (yes)\n",
                    super::plantuml::escape(&condition(rest))
                ));
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
            out.push_str(&format!(
                "else if ({}) then (yes)\n",
                super::plantuml::escape(&condition(&line))
            ));
            i += 1;
            continue;
        }
        if line.starts_with("else") {
            out.push_str("else (no)\n");
            i += 1;
            continue;
        }
        if line.starts_with("if (") {
            out.push_str(&format!(
                "if ({}) then (yes)\n",
                super::plantuml::escape(&condition(&line))
            ));
            stack.push("if");
            i += 1;
            continue;
        }
        if line.starts_with("while (") || line.starts_with("for (") {
            let cond = super::plantuml::escape(&condition(&line));
            out.push_str(&format!("while ({}) is (yes)\n", cond));
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
    true
}

/// The `catch (Type name) {` header, up to (and including) the closing `)`.
fn catch_header(line: &str) -> String {
    let mut close = first_code_byte(line, b'(')
        .and_then(|open| {
            let (code, _) = lexical_masks(line);
            matching_delimiter(line, &code, open, b'(', b')').map(|i| i + 1)
        })
        .unwrap_or(line.len());
    while line.as_bytes().get(close) == Some(&b'{') {
        close += 1;
    }
    line[..close.min(line.len())].trim().to_string()
}

/// Extract the parenthesised condition from `keyword (cond) ...`, matching the
/// outer parenthesis (conditions may contain nested calls).
fn condition(line: &str) -> String {
    let Some(open) = first_code_byte(line, b'(') else {
        return String::new();
    };
    let (code, _) = lexical_masks(line);
    let Some(close) = matching_delimiter(line, &code, open, b'(', b')') else {
        return String::new();
    };
    truncate_chars(line[open + 1..close].trim(), 45)
}

/// Parse a method source into a full PlantUML activity document, or `None`
/// when the body cannot be isolated.
pub fn document(source: &str, symbol: &str, title: &str) -> Option<String> {
    if source.len() > MAX_SOURCE_BYTES {
        return None;
    }
    let no_comments = strip_comments(source);
    let (open, close) = method_body(&no_comments, symbol)?;
    let head = no_comments[..open].trim();
    let body = &no_comments[open + 1..close];
    let mut steps = String::new();
    if !render_body(body, &mut steps) {
        return None;
    }
    if steps.trim().is_empty() {
        return None;
    }
    Some(format!(
        "@startuml\n!theme plain\n!pragma layout smetana\ntitle {}\nstart\n:Entry: {};\n{}stop\n@enduml\n",
        super::plantuml::escape(title),
        super::plantuml::escape(&signature(head, symbol)),
        steps
    ))
}

/// Convenience guard so the FLOW-based renderer can detect whether source
/// deepening produced anything usable without building the whole document.
pub fn usable(source: &str, symbol: &str) -> bool {
    if source.len() > MAX_SOURCE_BYTES {
        return false;
    }
    let no_comments = strip_comments(source);
    method_body(&no_comments, symbol)
        .and_then(|(o, c)| split_statements(&no_comments[o + 1..c]))
        .is_some_and(|statements| !statements.is_empty())
}

/// A lightweight view of the rendered steps, used by the render seam to decide
/// between source deepening and the FLOW renderer.
pub fn steps(source: &str, symbol: &str) -> Option<Value> {
    if source.len() > MAX_SOURCE_BYTES {
        return None;
    }
    let no_comments = strip_comments(source);
    let (open, close) = method_body(&no_comments, symbol)?;
    let mut out = String::new();
    if !render_body(&no_comments[open + 1..close], &mut out) {
        return None;
    }
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
            doc.contains(":Entry: changeTaskStatus（taskId, request）;\n"),
            "{doc}"
        );
        assert!(doc.contains(":taskInstance = null;"), "{doc}");
        assert!(
            doc.contains("if (anyTask.isPresent（）) then (yes)"),
            "{doc}"
        );
        assert!(doc.contains("else (no)"), "{doc}");
        assert!(
            doc.contains(":taskHandlingService.changeAnyTaskStatus（...）;"),
            "{doc}"
        );
        assert!(
            doc.contains(":taskInstance = taskHandlingService.changeTaskStatus（...）;"),
            "{doc}"
        );
        assert!(doc.contains("endif"), "{doc}");
        assert!(doc.contains("note right"), "{doc}");
        assert!(doc.contains("catch （Exception e）"), "{doc}");
        assert!(doc.contains(":return createResponse（...）;"), "{doc}");
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
    fn keep_args_respects_nested_parentheses() {
        // The closing `)` must be the outer one, not a nested call's paren.
        assert_eq!(
            keep_args("repo.closeErrorsForTask(taskInstance.getTaskId())"),
            "repo.closeErrorsForTask(taskInstance.getTaskId())"
        );
        assert_eq!(
            keep_args("svc.build(req.withId(taskId).withName(name))"),
            "svc.build(req.withId(taskId).withName(name))"
        );
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
        assert!(tree.starts_with("Entry: changeStatus()\n"), "{tree}");
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
        let stmts = split_statements(body).unwrap();
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
        assert!(tree.contains("log.info( \"count={}\", taskId"), "{tree}");
        assert!(!tree.contains("[W] log.info"), "{tree}");
        assert!(
            tree.contains("\"count={}\", taskId, task.getType()"),
            "{tree}"
        );
        assert!(!tree.contains("\"count = {}"), "{tree}");
        assert!(
            tree.contains("[D] if (task != null) then\n  svc.handle(task)"),
            "{tree}"
        );
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

    #[test]
    fn split_statements_flushes_single_line_block() {
        let body = "\
if (x) { foo(); }
bar();";
        let stmts = split_statements(body).unwrap();
        assert_eq!(stmts, vec!["if (x) {", "foo();", "}", "bar();"]);
    }

    #[test]
    fn tree_keeps_following_statement_after_single_line_block() {
        let src = "\
void m() {
    if (x) { foo(); }
    bar();
}";
        let tree = tree(src, "method:class:svc.TaskService#m()V").unwrap();
        assert!(tree.contains("foo()"), "{tree}");
        assert!(tree.contains("bar()"), "{tree}");
    }

    #[test]
    fn empty_and_compact_method_bodies_are_handled_without_dropping_first_byte() {
        let symbol = "method:class:svc.Service#run()V";
        assert!(document("void run() {}", symbol, "t").is_none());
        assert!(tree("void run() {}", symbol).is_none());

        let src = "void run(){if (ready) { act(); } save();}";
        let tree = tree(src, symbol).unwrap();
        assert!(tree.starts_with("Entry: run()\n"), "{tree}");
        assert!(
            tree.contains("[D] if (ready) then\n  act()\n[W] save()"),
            "{tree}"
        );
        let doc = document(src, symbol, "t").unwrap();
        assert!(doc.contains(":act（...）;"), "{doc}");
        assert!(doc.contains(":save（...）;"), "{doc}");
    }

    #[test]
    fn method_isolated_from_compact_class_and_annotation_wrappers() {
        let src = "@Trace(value = \"annotation() { not a body }\") class Wrapper { @Marker(value = \"run()\") public void run() { worker.execute(); } }";
        let symbol = "method:class:svc.Wrapper#run()V";
        let doc = document(src, symbol, "t").unwrap();
        assert!(doc.contains(":Entry: run;\n"), "{doc}");
        assert!(doc.contains(":worker.execute（...）;"), "{doc}");
        assert!(!doc.contains("Wrapper"), "{doc}");

        let ambiguous = "class Wrapper { void run() {} void run(int value) {} }";
        assert!(document(ambiguous, symbol, "t").is_none());
        assert!(tree(ambiguous, symbol).is_none());
    }

    #[test]
    fn comments_and_delimiters_inside_string_and_char_literals_are_preserved() {
        let src = r#"void run() {
    String url = "https://example.test/path";
    String markers = "/* text */ // still text";
    char close = '}';
    service.run();
}"#;
        let symbol = "method:class:svc.Service#run()V";
        let tree = tree(src, symbol).unwrap();
        assert!(
            tree.contains("url = \"https://example.test/path\""),
            "{tree}"
        );
        assert!(
            tree.contains("markers = \"/* text */ // still text\""),
            "{tree}"
        );
        assert!(tree.contains("close = '}'"), "{tree}");
        assert!(tree.contains("service.run()"), "{tree}");
        let doc = document(src, symbol, "t").unwrap();
        assert!(doc.contains("https://example.test/path"), "{doc}");
    }

    #[test]
    fn nested_multiline_call_arguments_keep_literal_parentheses_balanced() {
        let src =
            "void run() { service.call(\n    arg.nest(\"),\"),\n    inner(first, second)\n); }";
        let tree = tree(src, "method:class:svc.Service#run()V").unwrap();
        assert!(
            tree.contains("service.call( arg.nest(\"),\"), inner(first, second) )"),
            "{tree}"
        );
        assert_eq!(
            split_statements("service.call(arg.nest(\"),\"), inner(first, second));").unwrap(),
            vec!["service.call(arg.nest(\"),\"), inner(first, second));"]
        );
    }

    #[test]
    fn unicode_condition_is_truncated_without_splitting_a_character() {
        let repeated = "\u{0436}".repeat(80);
        let src = format!("void run() {{ if (name.equals(\"{repeated}\")) {{ act(); }} }}");
        let tree = tree(&src, "method:class:svc.Service#run()V").unwrap();
        let condition_line = tree.lines().find(|line| line.contains("[D] if")).unwrap();
        assert!(condition_line.ends_with("...) then"), "{tree}");
        assert!(tree.contains("act()"), "{tree}");
    }

    #[test]
    fn compact_control_spacing_is_normalized_and_sequential_loop_is_retained() {
        let src = "void run(){if(ready) { act(); } while(y) { tick(); }}";
        let tree = tree(src, "method:class:svc.Service#run()V").unwrap();
        assert!(tree.contains("[D] if (ready) then\n  act()"), "{tree}");
        assert!(tree.contains("[D] loop (y)\n  tick()"), "{tree}");
        let doc = document(src, "method:class:svc.Service#run()V", "t").unwrap();
        assert!(doc.contains("if (ready) then (yes)"), "{doc}");
        assert!(doc.contains("while (y) is (yes)"), "{doc}");
    }

    #[test]
    fn unbraced_control_body_falls_back_instead_of_losing_steps() {
        let src = "void run() { if (ready) act(); save(); }";
        let symbol = "method:class:svc.Service#run()V";
        assert!(tree(src, symbol).is_none());
        assert!(document(src, symbol, "t").is_none());
    }
}
