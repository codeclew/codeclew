//! Explicit, source-bound process steps. These declarations never grant compiler
//! call-target or runtime authority. Only complete retained callable spans are
//! admitted to the shared source section; rendering never opens a checkout.
use super::{
    check::Check,
    invalid,
    model::{Entrypoint, Source},
    source_steps, store,
};
use crate::error::ClewError;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Thread {
    pub scope: String,
    pub root_method: String,
    pub methods: Vec<Method>,
    pub steps: Vec<Step>,
    pub boundaries: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Method {
    pub id: String,
    pub label: String,
    pub service: String,
    pub entrypoint: String,
    pub source_id: String,
    pub text_digest: String,
}
/// Presentation labels are declarations; they do not prove control-flow semantics.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum StepKind {
    #[default]
    Step,
    Action,
    Decision,
    Alternative,
    Error,
    Boundary,
}
impl StepKind {
    fn is_default(&self) -> bool {
        *self == Self::Step
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Citation {
    pub method: String,
    pub start_line: u64,
    pub end_line: u64,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BranchMode {
    IfElse,
    FirstMatch,
    Independent,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BranchGroup {
    pub mode: BranchMode,
    pub arms: Vec<BranchArm>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BranchArm {
    pub id: String,
    /// None means the explicit default arm; it must be last and unguarded.
    pub condition: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard: Option<Citation>,
    pub steps: Vec<Step>,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EffectKind {
    StateAssignment,
    StorageCall,
    MessageSend,
    ExternalCall,
    Return,
    Throw,
    TransactionScope,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Effect {
    pub id: String,
    pub kind: EffectKind,
    pub label: String,
    pub citation: Citation,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Step {
    #[serde(default, skip_serializing_if = "StepKind::is_default")]
    pub kind: StepKind,
    pub id: String,
    pub title: String,
    pub text: String,
    pub method: String,
    pub start_line: u64,
    pub end_line: u64,
    /// An explicitly declared target, never a resolved call inferred from a name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Step>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch_group: Option<BranchGroup>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<Effect>,
}
fn bounded(text: &str) -> bool {
    !text.trim().is_empty() && text.len() <= 4096
}
impl Thread {
    pub(super) fn validate(&self, participants: &[String]) -> Result<(), ClewError> {
        if !bounded(&self.scope)
            || self.methods.is_empty()
            || self.methods.len() > 64
            || self.steps.is_empty()
            || self.boundaries.len() > 64
            || self.boundaries.iter().any(|text| !bounded(text))
        {
            return Err(invalid("invalid declared process thread bounds"));
        }
        let mut ids = BTreeSet::new();
        for method in &self.methods {
            if !store::valid_id(&method.id)
                || !ids.insert(method.id.as_str())
                || !bounded(&method.label)
                || !participants.contains(&method.service)
                || !bounded(&method.entrypoint)
                || !bounded(&method.source_id)
                || !method
                    .text_digest
                    .strip_prefix("sha256:")
                    .is_some_and(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
            {
                return Err(invalid("invalid declared thread method binding"));
            }
        }
        if !ids.contains(self.root_method.as_str()) {
            return Err(invalid("declared thread root method is missing"));
        }
        fn walk(
            steps: &[Step],
            methods: &BTreeSet<&str>,
            ids: &mut BTreeSet<String>,
            count: &mut usize,
            depth: usize,
        ) -> Result<(), ClewError> {
            if depth > 16 {
                return Err(invalid("declared thread nesting exceeds 16"));
            }
            for step in steps {
                *count += 1;
                if *count > 256
                    || !store::valid_id(&step.id)
                    || !ids.insert(step.id.clone())
                    || !bounded(&step.title)
                    || !bounded(&step.text)
                    || !methods.contains(step.method.as_str())
                    || step
                        .target
                        .as_ref()
                        .is_some_and(|id| !methods.contains(id.as_str()))
                    || step.start_line == 0
                    || step.end_line < step.start_line
                {
                    return Err(invalid("invalid declared thread step or citation"));
                }
                if step.branch_group.is_some() && !step.children.is_empty() {
                    return Err(invalid(
                        "explicit branch groups cannot also have explanatory children",
                    ));
                }
                let valid_citation = |citation: &Citation| {
                    methods.contains(citation.method.as_str())
                        && citation.start_line > 0
                        && citation.end_line >= citation.start_line
                };
                for effect in &step.effects {
                    *count += 1;
                    if *count > 256
                        || !store::valid_id(&effect.id)
                        || !ids.insert(effect.id.clone())
                        || !bounded(&effect.label)
                        || !valid_citation(&effect.citation)
                    {
                        return Err(invalid("invalid declared effect or citation"));
                    }
                }
                if let Some(group) = &step.branch_group {
                    if group.arms.is_empty()
                        || group.arms.len() > 32
                        || (group.mode == BranchMode::IfElse && group.arms.len() != 2)
                    {
                        return Err(invalid("invalid declared branch group bounds"));
                    }
                    for (index, arm) in group.arms.iter().enumerate() {
                        *count += 1;
                        if *count > 256
                            || !store::valid_id(&arm.id)
                            || !ids.insert(arm.id.clone())
                            || arm.steps.is_empty()
                        {
                            return Err(invalid("invalid declared branch arm"));
                        }
                        match (&arm.condition, &arm.guard) {
                            (Some(condition), Some(guard))
                                if bounded(condition) && valid_citation(guard) => {}
                            (None, None)
                                if index + 1 == group.arms.len()
                                    && group.mode != BranchMode::Independent => {}
                            _ => {
                                return Err(invalid(
                                    "branch condition needs a guard citation; default must be last",
                                ));
                            }
                        }
                        if group.mode == BranchMode::IfElse
                            && (index == 0 && arm.condition.is_none()
                                || index == 1 && arm.condition.is_some())
                        {
                            return Err(invalid(
                                "IF_ELSE requires a guarded arm followed by a default",
                            ));
                        }
                        walk(&arm.steps, methods, ids, count, depth + 1)?;
                    }
                }
                walk(&step.children, methods, ids, count, depth + 1)?;
            }
            Ok(())
        }
        walk(&self.steps, &ids, &mut BTreeSet::new(), &mut 0, 0)
    }
}

/// A bounded syntax check for method-only Kotlin or Java spans. It establishes
/// source containment only, without interpreting conditions or resolving calls.
// Remove only complete leading declaration annotations. Other executable
// prefixes remain visible and are rejected, rather than included in source.
fn without_annotations(mut prefix: &str) -> Option<String> {
    let mut result = String::new();
    while !prefix.trim_start().is_empty() {
        prefix = prefix.trim_start();
        if let Some(rest) = prefix.strip_prefix('@') {
            let len = rest
                .bytes()
                .take_while(|b| b.is_ascii_alphanumeric() || matches!(*b, b'_' | b'.' | b':'))
                .count();
            if len == 0 {
                return None;
            }
            prefix = rest[len..].trim_start();
            if prefix.starts_with('(') {
                let mut depth = 0;
                let mut end = None;
                for (i, byte) in prefix.bytes().enumerate() {
                    if byte == b'(' {
                        depth += 1;
                    }
                    if byte == b')' {
                        depth -= 1;
                        if depth == 0 {
                            end = Some(i + 1);
                            break;
                        }
                    }
                }
                prefix = &prefix[end?..];
            }
        } else {
            let end = prefix.find(char::is_whitespace).unwrap_or(prefix.len());
            result.push_str(&prefix[..end]);
            result.push(' ');
            prefix = &prefix[end..];
        }
    }
    Some(result)
}

fn isolated_callable(source: &Source, entry: &Entrypoint) -> bool {
    let text = &source.text;
    if text.len() > 256 * 1024 {
        return false;
    }
    let name = if entry.symbol.starts_with("source:") {
        entry.symbol.rsplit('/').nth(1).unwrap_or("")
    } else {
        entry
            .symbol
            .split_once('#')
            .map(|(_, value)| value.split('(').next().unwrap_or(""))
            .unwrap_or("")
    };
    let (code, comments) = source_steps::lexical_masks(text);
    let mut live = text.as_bytes().to_vec();
    for (i, &active) in code.iter().enumerate() {
        if !active {
            live[i] = b' ';
        }
    }
    let live = String::from_utf8(live).expect("masking preserves UTF-8");
    // Closure includes annotations, parameters, expression lambdas and arrays.
    // Strings/comments are opaque; a clipped expression must not pass merely
    // because its retained text has a matching digest.
    let mut delimiters = Vec::new();
    for byte in live.bytes() {
        match byte {
            b'(' => delimiters.push(b')'),
            b'[' => delimiters.push(b']'),
            b'{' => delimiters.push(b'}'),
            b')' | b']' | b'}' if delimiters.pop() != Some(byte) => return false,
            _ => (),
        }
    }
    if !delimiters.is_empty() {
        return false;
    }
    let words: Vec<_> = live
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .filter(|w| !w.is_empty())
        .collect();
    if words
        .iter()
        .any(|w| matches!(*w, "class" | "interface" | "enum"))
    {
        return false;
    }
    let declaration = if source.file.ends_with(".kt") {
        if words.iter().filter(|&&w| w == "fun").count() != 1 {
            return false;
        }
        let Some((start, _)) = live.match_indices("fun").find(|&(i, _)| {
            !live
                .as_bytes()
                .get(i.wrapping_sub(1))
                .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
                && !live
                    .as_bytes()
                    .get(i + 3)
                    .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
        }) else {
            return false;
        };
        let Some(prefix) = without_annotations(&live[..start]) else {
            return false;
        };
        if !prefix.split_whitespace().all(|word| {
            matches!(
                word,
                "override"
                    | "public"
                    | "private"
                    | "protected"
                    | "internal"
                    | "suspend"
                    | "inline"
                    | "tailrec"
                    | "operator"
                    | "infix"
                    | "actual"
                    | "expect"
                    | "final"
                    | "open"
            )
        }) {
            return false;
        }
        start
    } else if source.file.ends_with(".java") {
        let Some(start) = live.find(name).filter(|_| !name.is_empty()) else {
            return false;
        };
        let Some(prefix) = without_annotations(&live[..start]) else {
            return false;
        };
        if prefix.contains(['{', '}', '(', ')', ';', '=']) {
            return false;
        }
        start
    } else {
        return false;
    };
    // Reject trailing operators in expression bodies, without mistaking an
    // opaque literal or comment for an incomplete expression.
    let mut uncommented = text.as_bytes().to_vec();
    for (i, &comment) in comments.iter().enumerate() {
        if comment {
            uncommented[i] = b' ';
        }
    }
    let uncommented = String::from_utf8(uncommented).expect("masking preserves UTF-8");
    if uncommented[declaration..]
        .trim_end()
        .ends_with([',', '.', '=', '+', '-', '*', '/', '&', '|', ':'])
    {
        return false;
    }
    // Locate the body from this declaration's parameter list, not from every
    // occurrence of the method name. A receiver call with the same name is not
    // a second declaration (and must not make a complete body ambiguous).
    let Some(open) = live[declaration..].find('(').map(|i| declaration + i) else {
        return false;
    };
    let header = live[declaration..open].trim_end();
    let Some(before_name) = header.strip_suffix(name).filter(|_| !name.is_empty()) else {
        return false;
    };
    if before_name
        .as_bytes()
        .last()
        .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
    {
        return false;
    }
    fn close(live: &str, open: usize, left: u8, right: u8) -> Option<usize> {
        let mut depth = 0;
        for (i, byte) in live.bytes().enumerate().skip(open) {
            if byte == left {
                depth += 1;
            }
            if byte == right {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
        }
        None
    }
    let Some(parameters_end) = close(&live, open, b'(', b')') else {
        return false;
    };
    let Some(body) = live
        .bytes()
        .enumerate()
        .skip(parameters_end + 1)
        .find(|(_, byte)| matches!(byte, b'{' | b'=' | b';'))
        .map(|(i, _)| i)
    else {
        return false;
    };
    match live.as_bytes()[body] {
        b'{' => close(&live, body, b'{', b'}').is_some_and(|end| live[end + 1..].trim().is_empty()),
        b'=' if source.file.ends_with(".kt") => !uncommented[body + 1..].trim().is_empty(),
        _ => false,
    }
}
fn bind(
    method: &Method,
    source: &Source,
    entry: &Entrypoint,
    revision: &str,
) -> Result<(), ClewError> {
    if source.id != method.source_id
        || source.service != method.service
        || source.revision != revision
        || source.text_digest != method.text_digest
        || source.text_digest != crate::canonical::hash_bytes(source.text.as_bytes())
        || entry.id != method.entrypoint
        || entry.service != method.service
        || entry.source_ids != [method.source_id.clone()]
        || source
            .end_line
            .checked_sub(source.start_line)
            .map(|n| n + 1)
            != Some(source.text.lines().count() as u64)
        || !isolated_callable(source, entry)
    {
        return Err(invalid(format!(
            "declared thread method {} lacks its exact complete retained callable source",
            method.id
        )));
    }
    Ok(())
}

pub(super) fn project(
    checked: &Check,
    subject: &str,
    root: &Value,
) -> Result<Option<Value>, ClewError> {
    let Some(id) = subject.strip_prefix("scenario:") else {
        return Ok(None);
    };
    let Some(definition) = checked.dependencies.get(&format!("process:{id}")) else {
        return Ok(None);
    };
    let scenario: super::model::Scenario =
        serde_json::from_value(definition.normalized["definition"].clone())
            .map_err(|_| invalid("captured process declaration is invalid"))?;
    let Some(details) = scenario.process else {
        return Ok(None);
    };
    let Some(thread) = details.thread else {
        return Ok(None);
    };
    thread.validate(&details.participants)?;
    let mut methods = BTreeMap::new();
    let mut bytes = 0;
    for method in &thread.methods {
        let service = checked
            .services
            .get(&method.service)
            .ok_or_else(|| invalid("thread source service is unavailable"))?;
        let entry = service
            .entrypoints
            .iter()
            .find(|e| e.id == method.entrypoint)
            .ok_or_else(|| invalid("thread callable entrypoint is unavailable"))?;
        let source = service
            .sources
            .get(&method.source_id)
            .ok_or_else(|| invalid("thread method source is unavailable"))?;
        bind(method, source, entry, &service.revision)?;
        if method.id == thread.root_method
            && (root["symbol"] != entry.symbol || root["service"] != method.service)
        {
            return Err(invalid(
                "declared thread root differs from the exact selected process root",
            ));
        }
        bytes += source.text.len();
        if bytes > 1024 * 1024 {
            return Err(invalid("thread source section exceeds 1 MiB"));
        }
        methods.insert(method.id.clone(), json!({"label":method.label,"entrypoint":entry.id,"symbol":entry.symbol,"source":source}));
    }
    fn citations(
        steps: &[Step],
        methods: &BTreeMap<String, Value>,
        used: &mut BTreeSet<String>,
    ) -> Result<(), ClewError> {
        fn check(
            citation: &Citation,
            methods: &BTreeMap<String, Value>,
            used: &mut BTreeSet<String>,
        ) -> Result<(), ClewError> {
            let source = &methods
                .get(&citation.method)
                .ok_or_else(|| invalid("citation method is missing"))?["source"];
            if citation.start_line < source["startLine"].as_u64().unwrap_or(u64::MAX)
                || citation.end_line > source["endLine"].as_u64().unwrap_or(0)
            {
                return Err(invalid(
                    "declared branch/effect citation escapes its retained method",
                ));
            }
            used.insert(citation.method.clone());
            Ok(())
        }
        for step in steps {
            for effect in &step.effects {
                check(&effect.citation, methods, used)?;
            }
            if let Some(group) = &step.branch_group {
                for arm in &group.arms {
                    if let Some(guard) = &arm.guard {
                        check(guard, methods, used)?;
                    }
                    citations(&arm.steps, methods, used)?;
                }
            }
            let source = &methods[&step.method]["source"];
            if step.start_line < source["startLine"].as_u64().unwrap_or(u64::MAX)
                || step.end_line > source["endLine"].as_u64().unwrap_or(0)
            {
                return Err(invalid(format!(
                    "thread step {} citation escapes its retained method",
                    step.id
                )));
            }
            used.insert(step.method.clone());
            if let Some(target) = &step.target {
                used.insert(target.clone());
            }
            citations(&step.children, methods, used)?;
        }
        Ok(())
    }
    let mut used = BTreeSet::new();
    citations(&thread.steps, &methods, &mut used)?;
    if used.len() != methods.len() {
        return Err(invalid("thread source section contains an unused method"));
    }
    Ok(Some(
        json!({"schema":"codeclew-declared-process-thread/1.0","status":"SOURCE_BOUND_DECLARATION","authority":"DECLARED_MEANING_EXACT_RETAINED_SOURCE_NOT_COMPILER_CALL_GRAPH","rootMethod":thread.root_method,"declaredBranchCoverage":"NOT_PROVEN","selectedEffectCompleteness":"NOT_PROVEN","scope":thread.scope,"methods":methods,"methodOrder":thread.methods.iter().map(|m| &m.id).collect::<Vec<_>>(),"steps":thread.steps,"boundaries":thread.boundaries}),
    ))
}

/// Markdown keeps authored interpretation and exact selected sources together.
/// Citation links open the HTML reader's original line range; they do not grant
/// call-target authority. Choose a fence longer than any source backtick run.
pub(super) fn markdown(thread: &Value, subject: &str, language: &str) -> String {
    if thread["status"] != "SOURCE_BOUND_DECLARATION" {
        return String::new();
    }
    let label = |en| super::reader::process_thread_text(language, en);
    let esc = super::render::escape;
    let page = subject.strip_prefix("scenario:").unwrap_or("");
    let mut out = format!(
        "\n## {}\n\n{}\n\n",
        label("Process steps"),
        label(
            "Declared reading order does not prove execution order. Call targets and delivery are not established."
        )
    );
    fn steps(out: &mut String, rows: &[Value], page: &str, depth: usize) {
        for row in rows {
            let text = |key: &str| row[key].as_str().unwrap_or("");
            out.push_str(&format!(
                "{}- **{}**: {} ([source](./{}.html#thread-source-{}-L{}), lines {}–{}).\n",
                "  ".repeat(depth),
                super::render::escape(text("title")),
                super::render::escape(text("text")),
                page,
                text("method"),
                row["startLine"],
                row["startLine"],
                row["endLine"],
            ));
            for effect in row["effects"].as_array().into_iter().flatten() {
                let c = &effect["citation"];
                out.push_str(&format!(
                    "{}- [{}] {} ([source](./{}.html#thread-source-{}-L{})).\n",
                    "  ".repeat(depth + 1),
                    super::render::escape(effect["kind"].as_str().unwrap_or("")),
                    super::render::escape(effect["label"].as_str().unwrap_or("")),
                    page,
                    c["method"].as_str().unwrap_or(""),
                    c["startLine"]
                ));
            }
            if let Some(group) = row.get("branchGroup") {
                out.push_str(&format!(
                    "{}- **Declared branches: {}**\n",
                    "  ".repeat(depth + 1),
                    super::render::escape(group["mode"].as_str().unwrap_or(""))
                ));
                for arm in group["arms"].as_array().into_iter().flatten() {
                    out.push_str(&format!(
                        "{}- **{}**\n",
                        "  ".repeat(depth + 2),
                        super::render::escape(arm["condition"].as_str().unwrap_or("Otherwise"))
                    ));
                    if let Some(guard) = arm.get("guard") {
                        out.push_str(&format!(
                            "{}- [guard source](./{}.html#thread-source-{}-L{})\n",
                            "  ".repeat(depth + 3),
                            page,
                            guard["method"].as_str().unwrap_or(""),
                            guard["startLine"]
                        ));
                    }
                    if let Some(rows) = arm["steps"].as_array() {
                        steps(out, rows, page, depth + 3);
                    }
                }
            }
            if let Some(children) = row["children"].as_array() {
                steps(out, children, page, depth + 1);
            }
        }
    }
    if let Some(rows) = thread["steps"].as_array() {
        steps(&mut out, rows, page, 0);
    }
    out.push_str(&format!(
        "\n### {}\n\n{}\n\n",
        label("Scope and boundaries"),
        esc(thread["scope"].as_str().unwrap_or(""))
    ));
    for boundary in thread["boundaries"].as_array().into_iter().flatten() {
        out.push_str(&format!("- {}\n", esc(boundary.as_str().unwrap_or(""))));
    }
    out.push_str(&format!(
        "\n<details><summary>{}</summary>\n\n",
        label("Selected thread sources")
    ));
    for id in thread["methodOrder"].as_array().into_iter().flatten() {
        let method = &thread["methods"][id.as_str().unwrap_or("")];
        let source = &method["source"];
        let text = source["text"].as_str().unwrap_or("");
        let fence = "`".repeat(
            text.split(|ch| ch != '`')
                .map(str::len)
                .max()
                .unwrap_or(0)
                .max(2)
                + 1,
        );
        out.push_str(&format!(
            "### {}\n\n{} · {}:{}–{} · {}\n\n{}\n{}\n{}\n\n",
            esc(method["label"].as_str().unwrap_or("")),
            esc(source["service"].as_str().unwrap_or("")),
            esc(source["file"].as_str().unwrap_or("")),
            source["startLine"],
            source["endLine"],
            esc(source["revision"].as_str().unwrap_or("")),
            fence,
            text,
            fence
        ));
    }
    out.push_str("</details>\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample(text: &str) -> (Method, Source, Entrypoint) {
        let source = Source {
            id: "svc-source".into(),
            service: "svc".into(),
            revision: "rev".into(),
            file: "Order.kt".into(),
            start_line: 10,
            end_line: 9 + text.lines().count() as u64,
            text: text.into(),
            text_digest: crate::canonical::hash_bytes(text.as_bytes()),
            evidence_digest: "evidence".into(),
            authority: "EXACT_SNAPSHOT_TEXT".into(),
            occurrence: None,
            url: None,
        };
        let method = Method {
            id: "create".into(),
            label: "Create order".into(),
            service: "svc".into(),
            entrypoint: "svc-create".into(),
            source_id: source.id.clone(),
            text_digest: source.text_digest.clone(),
        };
        let entry = Entrypoint {
            id: method.entrypoint.clone(),
            service: "svc".into(),
            symbol: "source:Order.kt/class:OrderService/create/hash".into(),
            kind: "FUNCTION".into(),
            trigger: json!({}),
            source_ids: vec![source.id.clone()],
            dependency_ids: vec![],
            boundaries: vec![],
        };
        (method, source, entry)
    }
    #[test]
    fn complete_callable_keeps_guards_else_errors_and_return() {
        let (method, source, entry) = sample(
            "fun create() {\n require(ready) { \"class fun create fake\" }\n if (enabled) save() else throw Error()\n return result\n}",
        );
        bind(&method, &source, &entry, "rev").unwrap();
        let (method, source, entry) = sample("fun create() = Order(status = supplied ?: PENDING)");
        bind(&method, &source, &entry, "rev").unwrap();
    }
    #[test]
    fn annotated_root_with_same_named_receiver_call_is_one_complete_declaration() {
        let (method, source, entry) = sample(
            "@Suppress(\"TooGenericExceptionCaught\")\nfun create(dto: Request, author: String? = null): Order {\n require(ready) { \"Author required\" }\n val created = repository.create(dto)\n val result = if (enabled) { update(created) } else { created }\n return result\n}",
        );
        bind(&method, &source, &entry, "rev").unwrap();
    }
    #[test]
    fn expression_callable_with_lambda_retains_the_entire_expression() {
        let (method, source, entry) = sample(
            "fun create() = Order(\n progress = latest?.let { Progress(it.rows) },\n id = latest?.id\n)",
        );
        bind(&method, &source, &entry, "rev").unwrap();
    }
    #[test]
    fn native_projection_rejects_citation_overflow_and_a_different_selected_root() {
        let (method, source, entry) = sample("fun create() {\n return order\n}");
        let thread = Thread {
            scope: "One order creation".into(),
            root_method: "create".into(),
            methods: vec![method],
            steps: vec![Step {
                kind: StepKind::Step,
                id: "return".into(),
                title: "Return order".into(),
                text: "Declared return".into(),
                method: "create".into(),
                start_line: 11,
                end_line: 11,
                target: None,
                children: vec![],
                branch_group: None,
                effects: vec![],
            }],
            boundaries: vec![],
        };
        let definition = json!({"schema":"codeclew-documentation-process/1.0","id":"order","title":"Order creation","summary":"Declared creation","root":{"service":"svc","selector":{"language":"kotlin","owner":"OrderService","name":"create"}},"process":{"scope":"Order creation","participants":["svc"],"trigger":"Request","outcomes":["Order"],"thread":thread}});
        let evidence = super::super::model::ServiceEvidence {
            schema: "evidence".into(),
            service: "svc".into(),
            revision: "rev".into(),
            service_digest: "digest".into(),
            extractor: "syntax".into(),
            runtime_mode: "source".into(),
            coverage: "SYNTAX".into(),
            boundaries: vec![],
            entrypoints: vec![entry.clone()],
            observations: BTreeMap::new(),
            sources: BTreeMap::from([(source.id.clone(), source)]),
            contracts: BTreeMap::new(),
        };
        let mut checked = Check {
            schema: "check".into(),
            input_digest: "input".into(),
            context_digest: "context".into(),
            services: BTreeMap::from([("svc".into(), evidence)]),
            unresolved: BTreeMap::new(),
            interactions: BTreeMap::new(),
            scenarios: BTreeMap::new(),
            dependencies: BTreeMap::from([(
                "process:order".into(),
                super::super::model::Observation {
                    id: "process:order".into(),
                    kind: "PROCESS_DEFINITION".into(),
                    service: "svc".into(),
                    symbol: "order".into(),
                    normalized: json!({"definition":definition}),
                    digest: "digest".into(),
                    source_ids: vec![],
                },
            )]),
            source_inputs: None,
            composition: None,
        };
        let root = json!({"service":"svc","symbol":entry.symbol});
        let page = project(&checked, "scenario:order", &root).unwrap().unwrap();
        assert_eq!(page["status"], "SOURCE_BOUND_DECLARATION");
        assert!(
            project(
                &checked,
                "scenario:order",
                &json!({"service":"svc","symbol":"another overload"})
            )
            .is_err()
        );
        checked
            .dependencies
            .get_mut("process:order")
            .unwrap()
            .normalized["definition"]["process"]["thread"]["steps"][0]["effects"] = json!([{ "id":"selected-return", "kind":"RETURN", "label":"Return the result", "citation":{"method":"create","startLine":11,"endLine":999} }]);
        assert!(project(&checked, "scenario:order", &root).is_err());
        checked
            .dependencies
            .get_mut("process:order")
            .unwrap()
            .normalized["definition"]["process"]["thread"]["steps"][0]["effects"][0]["citation"]
            ["endLine"] = json!(11);
        assert!(project(&checked, "scenario:order", &root).is_ok());
        checked
            .dependencies
            .get_mut("process:order")
            .unwrap()
            .normalized["definition"]["process"]["thread"]["steps"][0]["endLine"] = json!(999);
        assert!(project(&checked, "scenario:order", &root).is_err());
    }
    #[test]
    fn rejects_changed_revision_forged_digest_unrelated_class_or_method_and_clipped_body() {
        let (method, mut source, entry) = sample("fun create() { return result }");
        assert!(bind(&method, &source, &entry, "other").is_err());
        source.text.push_str(" // changed");
        assert!(bind(&method, &source, &entry, "rev").is_err());
        for text in [
            "class Order { fun create() { return result } }",
            "fun create() { return result }\nfun unrelated() {}",
            "fun create() {\n return result",
            "fun create() = Order(\n status = supplied ?: PENDING,",
            "val unrelated = loadAllUsers()\nfun create() { return result }",
        ] {
            let (m, s, e) = sample(text);
            assert!(bind(&m, &s, &e, "rev").is_err(), "{text}");
        }
    }
    #[test]
    fn rejects_an_unrelated_java_member_before_the_selected_method() {
        let (method, mut source, mut entry) =
            sample("void unrelated() {}\nvoid create() { return; }");
        source.file = "Order.java".into();
        entry.symbol = "method:class:OrderService#create()V".into();
        assert!(bind(&method, &source, &entry, "rev").is_err());
    }
    #[test]
    fn rejects_duplicate_steps_and_unknown_target_even_when_nested() {
        let (method, _, _) = sample("fun create() = Order()");
        let step = Step {
            kind: StepKind::Step,
            id: "step".into(),
            title: "Create".into(),
            text: "Declared interpretation".into(),
            method: "create".into(),
            start_line: 10,
            end_line: 10,
            target: None,
            children: vec![],
            branch_group: None,
            effects: vec![],
        };
        let mut thread = Thread {
            scope: "One order creation".into(),
            root_method: "create".into(),
            methods: vec![method],
            steps: vec![step.clone()],
            boundaries: vec![],
        };
        thread.validate(&["svc".into()]).unwrap();
        thread.steps[0].children.push(step);
        assert!(thread.validate(&["svc".into()]).is_err());
        thread.steps[0].children.clear();
        thread.steps[0].target = Some("unselected".into());
        assert!(thread.validate(&["svc".into()]).is_err());
    }

    #[test]
    fn old_steps_have_no_inferred_control_kind_and_unknown_kinds_fail_closed() {
        let value = json!({"id":"step","title":"If ready","text":"A declared condition", "method":"create","startLine":10,"endLine":10});
        let step: Step = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(step.kind, StepKind::Step);
        assert!(serde_json::to_value(&step).unwrap().get("kind").is_none());
        let mut decision = value.clone();
        decision["kind"] = json!("DECISION");
        assert_eq!(
            serde_json::from_value::<Step>(decision).unwrap().kind,
            StepKind::Decision
        );
        let mut unsupported = value;
        unsupported["kind"] = json!("COMPILER_PROVEN_BRANCH");
        assert!(serde_json::from_value::<Step>(unsupported).is_err());
    }

    #[test]
    fn explicit_branches_require_guards_unique_ids_and_exclusive_default_shape() {
        let (method, _, _) = sample("fun create() = result");
        let mut thread: Thread = serde_json::from_value(json!({"scope":"Bounded creation","rootMethod":"create","methods":[method],"boundaries":[],"steps":[{"id":"decision","title":"Choose result","text":"Declared choice","method":"create","startLine":10,"endLine":10,"branchGroup":{"mode":"IF_ELSE","arms":[{"id":"yes","condition":"ready","guard":{"method":"create","startLine":10,"endLine":10},"steps":[{"id":"yes-step","title":"Return prepared result","text":"Return when ready","method":"create","startLine":10,"endLine":10}]},{"id":"no","condition":null,"steps":[{"id":"no-step","title":"Return fallback","text":"Return otherwise","method":"create","startLine":10,"endLine":10}]}]}}]})).unwrap();
        assert!(thread.validate(&["svc".into()]).is_ok());
        let old = thread.clone();
        thread.steps[0].branch_group.as_mut().unwrap().arms[0].guard = None;
        assert!(thread.validate(&["svc".into()]).is_err());
        thread = old.clone();
        thread.steps[0]
            .branch_group
            .as_mut()
            .unwrap()
            .arms
            .swap(0, 1);
        assert!(thread.validate(&["svc".into()]).is_err());
        thread = old.clone();
        thread.steps[0].branch_group.as_mut().unwrap().arms[1].id = "decision".into();
        assert!(thread.validate(&["svc".into()]).is_err());
        thread = old;
        thread.steps[0].children = thread.steps[0].branch_group.as_ref().unwrap().arms[0]
            .steps
            .clone();
        assert!(thread.validate(&["svc".into()]).is_err());
    }

    #[test]
    fn markdown_retains_source_bytes_and_fences_embedded_backticks() {
        let code = "fun create() = \"```<script>\"";
        let thread = json!({"status":"SOURCE_BOUND_DECLARATION","scope":"One method", "steps":[{"id":"step","title":"Return","text":"Declared return", "method":"create","startLine":10,"endLine":10}],"boundaries":["Delivery unknown"],"methodOrder":["create"],"methods":{"create":{"label":"Create","source":{"service":"svc","file":"Order.kt","revision":"rev","startLine":10,"endLine":10,"text":code}}}});
        let output = markdown(&thread, "scenario:order", "en");
        assert!(output.contains("./order.html#thread-source-create-L10"));
        assert!(output.contains(&format!("````\n{code}\n````")));
        assert_eq!(output.matches("<details>").count(), 1);
        assert!(output.contains("Delivery unknown"));
        assert_ne!(markdown(&thread, "scenario:order", "ru"), output);
        assert!(markdown(&Value::Null, "scenario:order", "en").is_empty());
    }
}
