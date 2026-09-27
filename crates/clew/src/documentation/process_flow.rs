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

fn evidence_comment_text(symbol: &str) -> String {
    symbol
        .chars()
        .map(|ch| match ch {
            '\r' | '\n' | '\u{0085}' | '\u{2028}' | '\u{2029}' => ' ',
            ch if ch.is_control() => ' ',
            ch => ch,
        })
        .collect()
}

/// Small method-local vocabulary shared by source and FLOW projections.
/// `Gap` is evidence about a limit, never a step that claims execution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ProjectionStep {
    Action {
        diagram: String,
        tree: String,
        category: Option<&'static str>,
    },
    If(String),
    Else,
    End,
    MethodReturn(String),
    Gap(String),
}

#[derive(Clone, Debug)]
pub(crate) struct Projection {
    pub(crate) symbol: String,
    pub(crate) entry: String,
    pub(crate) steps: Vec<ProjectionStep>,
    pub(crate) evidence_gaps: Vec<String>,
    pub(crate) origin: &'static str,
    /// True only after the root producer and its control envelope qualify.
    pub(crate) source_eligible: bool,
    /// Expression-only FLOW gaps permit exact SOURCE text, but not an ordered
    /// FLOW-only rendering of calls inside the expression.
    pub(crate) flow_fallback_gap: Option<String>,
    /// A whole-method uncertainty that must suppress all causal steps.
    pub(crate) noncausal: Option<String>,
}

pub(crate) struct ValidatedProjection {
    pub(crate) projection: Projection,
    pub(crate) evidence_gaps: Vec<String>,
}

impl ValidatedProjection {
    pub(crate) fn source_eligible(&self) -> bool {
        self.projection.source_eligible
    }
}

pub(crate) struct RenderedProjection {
    pub(crate) puml: String,
    pub(crate) tree: String,
    pub(crate) origin: &'static str,
}

impl Projection {
    pub(crate) fn source(symbol: &str, entry: String, steps: Vec<ProjectionStep>) -> Self {
        Self {
            symbol: symbol.to_string(),
            entry,
            steps,
            evidence_gaps: Vec::new(),
            origin: "source",
            source_eligible: false,
            flow_fallback_gap: None,
            noncausal: None,
        }
    }
}

fn qualified_documentation_authority(documentation: &Value) -> bool {
    documentation["authority"] == "JAVAC_SOURCE_STRUCTURE"
}

fn control_critical_boundary(code: &str) -> bool {
    // Only these two expression-level boundaries leave the surrounding method
    // outline useful as a single source statement. Unknown codes stay
    // conservative until a focused capability is qualified.
    !matches!(
        code,
        "SHORT_CIRCUIT_FLOW_REQUIRES_SOURCE_REVIEW" | "TERNARY_FLOW_REQUIRES_SOURCE_REVIEW"
    ) && !code.starts_with("HTTP_BOUNDARY:")
}

fn boundary_codes(documentation: &Value, events: &Value) -> Vec<String> {
    let mut codes = Vec::new();
    if let Some(rows) = documentation["boundaries"].as_array() {
        for row in rows {
            if let Some(code) = row.as_str().filter(|code| !code.is_empty())
                && !codes.iter().any(|old| old == code)
            {
                codes.push(code.to_owned());
            }
        }
    }
    if let Some(rows) = events.as_array() {
        for row in rows {
            if let Some(boundary) = row.pointer("/http/boundary").and_then(Value::as_str) {
                let code = format!("HTTP_BOUNDARY:{boundary}");
                if !codes.contains(&code) {
                    codes.push(code);
                }
            }
        }
        for row in rows.iter().filter(|row| row["kind"] == "BOUNDARY") {
            if let Some(code) = row["code"].as_str().filter(|code| !code.is_empty()) {
                if !codes.iter().any(|old| old == code) {
                    codes.push(code.to_owned());
                }
            } else if codes.is_empty() {
                codes.push("FLOW_BOUNDARY_UNSPECIFIED".into());
            }
        }
    }
    codes
}

/// Project compiler FLOW without repairing its structure. The root
/// documentation authority and every boundary are considered before a
/// return is allowed to terminate a rendered branch.
pub(crate) fn project_flow(events: &Value, documentation: &Value, symbol: &str) -> Projection {
    let mut projection = Projection {
        symbol: symbol.to_string(),
        entry: method_label(symbol),
        steps: Vec::new(),
        evidence_gaps: Vec::new(),
        origin: "flow",
        source_eligible: false,
        flow_fallback_gap: None,
        noncausal: None,
    };
    if !qualified_documentation_authority(documentation) {
        projection
            .evidence_gaps
            .push("FLOW_AUTHORITY_NOT_QUALIFIED".into());
        projection.noncausal = Some("FLOW_AUTHORITY_NOT_QUALIFIED".into());
        return projection;
    }
    let Some(rows) = events.as_array() else {
        projection
            .evidence_gaps
            .push("FLOW_EVENTS_MISSING_OR_MALFORMED".into());
        projection.noncausal = Some("FLOW_EVENTS_MISSING_OR_MALFORMED".into());
        return projection;
    };
    if rows.is_empty() {
        projection.evidence_gaps.push("FLOW_EVENTS_EMPTY".into());
        projection.noncausal = Some("FLOW_EVENTS_EMPTY".into());
        return projection;
    }

    let boundaries = boundary_codes(documentation, events);
    if let Some(code) = boundaries
        .iter()
        .find(|code| control_critical_boundary(code))
        .cloned()
    {
        let detail = boundaries.join(",");
        projection.evidence_gaps.extend(boundaries);
        projection.noncausal = Some(format!(
            "CONTROL_BOUNDARY:{code}; EVIDENCE_BOUNDARIES:{detail}"
        ));
        return projection;
    }
    projection.evidence_gaps = boundaries;
    if projection.evidence_gaps.iter().any(|code| {
        matches!(
            code.as_str(),
            "SHORT_CIRCUIT_FLOW_REQUIRES_SOURCE_REVIEW" | "TERNARY_FLOW_REQUIRES_SOURCE_REVIEW"
        )
    }) {
        projection.flow_fallback_gap = Some("FLOW_EXPRESSION_ORDER_UNQUALIFIED".into());
    }

    let mut meaningful = false;
    let mut invalid = None;
    for row in rows {
        let kind = row["kind"].as_str().unwrap_or("");
        match kind {
            "IF" => {
                let condition = row["condition"].as_str().unwrap_or("");
                if condition.trim().is_empty() {
                    invalid = Some("IF_CONDITION_MISSING".to_string());
                    break;
                }
                meaningful = true;
                projection
                    .steps
                    .push(ProjectionStep::If(condition.to_string()));
            }
            "ELSE" => {
                if !row["condition"].as_str().unwrap_or("").trim().is_empty() {
                    invalid = Some("FLOW_ELSE_CONDITION_UNSUPPORTED".to_string());
                    break;
                }
                projection.steps.push(ProjectionStep::Else);
            }
            "ELSEIF" => {
                let condition = row["condition"].as_str().unwrap_or("");
                if condition.trim().is_empty() {
                    invalid = Some("ELSEIF_CONDITION_MISSING".to_string());
                    break;
                }
                projection.steps.push(ProjectionStep::Else);
                projection
                    .steps
                    .push(ProjectionStep::If(condition.to_string()));
                meaningful = true;
            }
            "END" => projection.steps.push(ProjectionStep::End),
            "CALL" | "CONSTRUCT" => {
                let target = row["target"].as_str().unwrap_or("");
                if target.is_empty() || row["targetStatus"] == "UNRESOLVED" {
                    invalid = Some("FLOW_CALL_TARGET_UNRESOLVED".to_string());
                    break;
                }
                // HTTP annotations mark an attempted client call even when
                // the receiver is a framework type. Keep that exact CALL
                // visible; boundary metadata is evidence, not delivery proof.
                let has_http_metadata =
                    row["http"].as_object().is_some_and(|http| !http.is_empty());
                if has_http_metadata || !is_framework_symbol(target) {
                    let label = method_label(target);
                    let category = step_kind(kind, target);
                    projection.steps.push(ProjectionStep::Action {
                        diagram: label.clone(),
                        tree: label,
                        category,
                    });
                }
                meaningful = true;
            }
            "RETURN" => {
                projection
                    .steps
                    .push(ProjectionStep::MethodReturn(String::new()));
                meaningful = true;
            }
            "BOUNDARY" => {
                let code = row["code"].as_str().unwrap_or_else(|| {
                    projection
                        .evidence_gaps
                        .first()
                        .map(String::as_str)
                        .unwrap_or("FLOW_BOUNDARY_UNSPECIFIED")
                });
                if control_critical_boundary(code) {
                    invalid = Some(code.to_string());
                    break;
                }
                projection.steps.push(ProjectionStep::Gap(code.to_string()));
            }
            "STATEMENT" | "LOCAL" => projection
                .steps
                .push(ProjectionStep::Gap(format!("FLOW_{kind}_TEXT_UNAVAILABLE"))),
            "LOOP" => {
                invalid = Some("FLOW_LOOP_UNSUPPORTED_IN_M1".to_string());
                break;
            }
            "THROW" | "TRY" | "CATCH" | "FINALLY" | "SWITCH" | "BREAK" | "CONTINUE"
            | "DEFERRED" => {
                invalid = Some(format!("FLOW_{kind}_CONTROL_UNSUPPORTED"));
                break;
            }
            "" => {
                invalid = Some("FLOW_EVENT_KIND_MISSING".to_string());
                break;
            }
            unknown => {
                invalid = Some(format!("FLOW_EVENT_UNSUPPORTED:{unknown}"));
                break;
            }
        }
    }
    if let Some(reason) = invalid {
        projection.noncausal = Some(reason);
        projection.source_eligible = false;
        return projection;
    }
    projection.source_eligible = meaningful
        && documentation["authority"] == "JAVAC_SOURCE_STRUCTURE"
        && !projection
            .evidence_gaps
            .iter()
            .any(|code| control_critical_boundary(code));
    projection
}

/// Validate branch delimiters once, then let both renderers consume the same
/// accepted prefix. Invalid or truncated structure becomes a non-causal gap.
pub(crate) fn validate_projection(mut projection: Projection) -> ValidatedProjection {
    let evidence_gaps = projection.evidence_gaps.clone();
    if let Some(reason) = projection.noncausal.take() {
        projection.steps.clear();
        let reason = if projection.evidence_gaps.is_empty() {
            reason
        } else {
            format!(
                "{reason}; EVIDENCE_BOUNDARIES:{}",
                projection.evidence_gaps.join(",")
            )
        };
        projection.steps.push(ProjectionStep::Gap(reason));
        projection.source_eligible = false;
        return ValidatedProjection {
            projection,
            evidence_gaps,
        };
    }
    // Validate the original branch envelope before converting expression
    // FLOW gaps to evidence-only output. SOURCE may deepen an opaque short-
    // circuit expression, but it cannot repair a missing END or malformed
    // branch around that expression.
    let mut stack: Vec<bool> = Vec::new(); // one `true` per if that already has an else
    let mut invalid = None;
    for step in &projection.steps {
        match step {
            ProjectionStep::If(condition) if condition.trim().is_empty() => {
                invalid = Some("IF_CONDITION_MISSING".to_string());
                break;
            }
            ProjectionStep::If(_) => stack.push(false),
            ProjectionStep::Else => match stack.last_mut() {
                Some(has_else) if !*has_else => *has_else = true,
                _ => {
                    invalid = Some("ELSE_WITHOUT_OPEN_IF_OR_DUPLICATE_ELSE".to_string());
                    break;
                }
            },
            ProjectionStep::End => {
                if stack.pop().is_none() {
                    invalid = Some("END_WITHOUT_OPEN_IF".to_string());
                    break;
                }
            }
            _ => {}
        }
    }
    if invalid.is_none() && !stack.is_empty() {
        invalid = Some("IF_WITHOUT_END".to_string());
    }
    if let Some(reason) = invalid {
        projection.steps.clear();
        projection
            .steps
            .push(ProjectionStep::Gap(format!("MALFORMED_CONTROL:{reason}")));
        projection.source_eligible = false;
        return ValidatedProjection {
            projection,
            evidence_gaps,
        };
    }

    if let Some(reason) = projection.flow_fallback_gap.take() {
        projection.steps.clear();
        let reason = if evidence_gaps.is_empty() {
            reason
        } else {
            format!("{reason}; EVIDENCE_BOUNDARIES:{}", evidence_gaps.join(","))
        };
        projection.steps.push(ProjectionStep::Gap(reason));
        return ValidatedProjection {
            projection,
            evidence_gaps,
        };
    }
    if !projection.evidence_gaps.is_empty() {
        let mut gaps = projection
            .evidence_gaps
            .drain(..)
            .map(ProjectionStep::Gap)
            .collect::<Vec<_>>();
        gaps.append(&mut projection.steps);
        projection.steps = gaps;
    }
    ValidatedProjection {
        projection,
        evidence_gaps,
    }
}

/// Render tree and PlantUML from the same validated projection.
pub(crate) fn render_validated(
    validated: ValidatedProjection,
    title: &str,
) -> Option<RenderedProjection> {
    let projection = validated.projection;
    if projection.steps.is_empty() {
        return None;
    }
    let mut tree = format!("Entry: {}\n", projection.entry);
    let mut steps_puml = String::new();
    let causal = projection
        .steps
        .iter()
        .any(|step| !matches!(step, ProjectionStep::Gap(_)));
    let mut depth = 0usize;
    for step in &projection.steps {
        let indent = "  ".repeat(depth);
        match step {
            ProjectionStep::Action {
                diagram,
                tree: label,
                category,
            } => {
                let label = flatten_tree_text(label);
                tree.push_str(&format!(
                    "{indent}{}{label}\n",
                    category.map(|c| format!("[{c}] ")).unwrap_or_default()
                ));
                steps_puml.push_str(&format!(":{};\n", super::plantuml::escape(diagram)));
            }
            ProjectionStep::If(condition) => {
                let display = truncate_chars(condition, 72, 69);
                tree.push_str(&format!(
                    "{indent}[D] if ({}) then\n",
                    flatten_tree_text(condition)
                ));
                steps_puml.push_str(&format!(
                    "if ({}) then (yes)\n",
                    super::plantuml::escape(&display)
                ));
                depth += 1;
            }
            ProjectionStep::Else => {
                depth = depth.saturating_sub(1);
                tree.push_str(&format!("{}else\n", "  ".repeat(depth)));
                steps_puml.push_str("else (no)\n");
                depth += 1;
            }
            ProjectionStep::End => {
                depth = depth.saturating_sub(1);
                steps_puml.push_str("endif\n");
            }
            ProjectionStep::MethodReturn(value) => {
                let line = if value.is_empty() {
                    "return".to_string()
                } else {
                    format!("return {value}")
                };
                tree.push_str(&format!("{indent}{}\n", flatten_tree_text(&line)));
                let display = truncate_chars(&line, 120, 117);
                steps_puml.push_str(&format!(":{};\nstop\n", super::plantuml::escape(&display)));
            }
            ProjectionStep::Gap(reason) => {
                let reason = flatten_tree_text(reason);
                tree.push_str(&format!("{indent}... (not established: {reason})\n"));
                steps_puml.push_str(&format!(
                    "note right\n  {} (not established)\nend note\n",
                    super::plantuml::escape(&reason)
                ));
            }
        }
    }
    // A terminal final return already contains its branch-local `stop`.
    let all_reachable_paths_return = sequence_terminates(&projection.steps);
    let mut puml = format!(
        "@startuml\n!theme plain\n!pragma layout smetana\ntitle {}\n",
        super::plantuml::escape(title)
    );
    if causal {
        puml.push_str(&format!(
            "start\n:Entry: {};\n' evidence: {}\n{}",
            super::plantuml::escape(&projection.entry),
            evidence_comment_text(&projection.symbol),
            steps_puml
        ));
        if !all_reachable_paths_return {
            puml.push_str("stop\n");
        }
    } else {
        // Evidence-only diagrams make no start/stop or execution claim.
        let gaps = projection
            .steps
            .iter()
            .filter_map(|step| match step {
                ProjectionStep::Gap(reason) => Some(reason.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        puml.push_str(&format!(
            "note as Evidence\n{}\nend note\n",
            super::plantuml::escape(&gaps)
        ));
    }
    puml.push_str("@enduml\n");
    Some(RenderedProjection {
        puml,
        tree,
        origin: projection.origin,
    })
}

fn flatten_tree_text(text: &str) -> String {
    text.chars()
        .map(|ch| match ch {
            '\r' | '\n' | '\u{0085}' | '\u{2028}' | '\u{2029}' => ' ',
            ch if ch.is_control() => ' ',
            ch => ch,
        })
        .collect()
}

fn sequence_terminates(steps: &[ProjectionStep]) -> bool {
    let mut index = 0;
    let mut last_terminates = false;
    while index < steps.len() {
        match &steps[index] {
            ProjectionStep::MethodReturn(_) => {
                last_terminates = true;
                index += 1;
            }
            ProjectionStep::If(_) => {
                let Some((then_start, then_end, else_range, after)) = if_regions(steps, index)
                else {
                    return false;
                };
                let Some((else_start, else_end)) = else_range else {
                    last_terminates = false;
                    index = after;
                    continue;
                };
                last_terminates = sequence_terminates(&steps[then_start..then_end])
                    && sequence_terminates(&steps[else_start..else_end]);
                index = after;
            }
            ProjectionStep::Action { .. } | ProjectionStep::Gap(_) => {
                last_terminates = false;
                index += 1;
            }
            ProjectionStep::Else | ProjectionStep::End => return false,
        }
    }
    last_terminates
}

type IfRegionRanges = (usize, usize, Option<(usize, usize)>, usize);

fn if_regions(steps: &[ProjectionStep], start: usize) -> Option<IfRegionRanges> {
    let mut depth = 1usize;
    let mut else_index = None;
    for (index, step) in steps.iter().enumerate().skip(start + 1) {
        match step {
            ProjectionStep::If(_) => depth += 1,
            ProjectionStep::Else if depth == 1 => {
                if else_index.replace(index).is_some() {
                    return None;
                }
            }
            ProjectionStep::End => {
                depth -= 1;
                if depth == 0 {
                    let then_end = else_index.unwrap_or(index);
                    let else_range = else_index.map(|else_index| (else_index + 1, index));
                    return Some((start + 1, then_end, else_range, index + 1));
                }
            }
            _ => {}
        }
    }
    None
}

/// True when a `method:class:<package>...` target belongs to the runtime/JDK
/// rather than the service domain. Framework plumbing (collections, Optional,
/// Spring, JDBC) adds noise to a process diagram and is filtered out.
fn is_framework_symbol(symbol: &str) -> bool {
    let package = symbol.strip_prefix("method:class:").unwrap_or(symbol);
    // These calls are meaningful process leaves. Their presence means a
    // source call attempt only, not persistence or remote delivery.
    if package.starts_with("org.springframework.data.repository.")
        || package.starts_with("org.springframework.web.client.RestTemplate#")
        || package.starts_with("org.springframework.web.reactive.function.client.")
    {
        return false;
    }
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
    let projection = project_flow(events, &Value::Null, symbol);
    render_validated(validate_projection(projection), "Method flow").map(|rendered| rendered.puml)
}

/// Produce a full PlantUML activity document for a flow, or `None` if empty.
pub fn document(events: &Value, symbol: &str, title: &str) -> Option<String> {
    let projection = project_flow(events, &Value::Null, symbol);
    render_validated(validate_projection(projection), title).map(|rendered| rendered.puml)
}

/// Render a flow as an indented pseudocode tree (readable in a `<pre>` block)
/// instead of a diagram image. Root line carries the method, then each step is
/// indented by block depth; branches and loops open a level, `else` continues
/// the current block.
pub fn tree(events: &Value, symbol: &str) -> Option<String> {
    let projection = project_flow(events, &Value::Null, symbol);
    render_validated(validate_projection(projection), "Method flow").map(|rendered| rendered.tree)
}

fn truncate_chars(text: &str, max_chars: usize, keep_chars: usize) -> String {
    if text.chars().count() > max_chars {
        format!("{}...", text.chars().take(keep_chars).collect::<String>())
    } else {
        text.to_string()
    }
}

/// Classify a method name as write (`W`) or read (`R`) by verb prefix.
/// Shared by the FLOW tree (`step_kind`) and the source-deepened tree
/// (`source_steps`).
pub(crate) fn method_write_read(name: &str) -> Option<&'static str> {
    const WRITE: &[&str] = &[
        "set", "update", "save", "add", "remove", "close", "delete", "builder", "build", "<init>",
    ];
    const READ: &[&str] = &["get", "find", "is", "has", "contains", "load"];
    if WRITE.iter().any(|p| name.starts_with(p)) {
        Some("W")
    } else if READ.iter().any(|p| name.starts_with(p)) {
        Some("R")
    } else {
        None
    }
}

/// Classify a flow step into a readability category for the pseudocode tree:
/// `W` (write / state change), `R` (read), `D` (decision). Returns `None` for
/// control-flow or unclassifiable steps, which are rendered without a prefix.
fn step_kind(kind: &str, target: &str) -> Option<&'static str> {
    if matches!(kind, "IF" | "LOOP") {
        return Some("D");
    }
    let method = target.rsplit('#').next().unwrap_or(target);
    let method = method.split('(').next().unwrap_or(method);
    method_write_read(method)
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    fn qualified(events: Value, boundaries: Value) -> super::ValidatedProjection {
        let symbol = "method:class:svc.Checkout#checkout()Ljava/lang/String;";
        let documentation = json!({
            "authority":"JAVAC_SOURCE_STRUCTURE",
            "boundaries":boundaries
        });
        super::validate_projection(super::project_flow(&events, &documentation, symbol))
    }

    fn gap_only(rendered: &super::RenderedProjection, code: &str) {
        assert!(rendered.tree.contains(code), "{}", rendered.tree);
        assert!(rendered.puml.contains(code), "{}", rendered.puml);
        assert!(!rendered.puml.contains("start\n"), "{}", rendered.puml);
        assert!(!rendered.puml.contains("stop\n"), "{}", rendered.puml);
    }

    #[test]
    fn checkout_guard_return_is_branch_local_and_final_return_terminates() {
        let events = json!([
            {"kind":"IF","condition":"!hasPositiveQuantity(request)"},
            {"kind":"RETURN"},
            {"kind":"END"},
            {"kind":"CALL","resolution":"COMPILER_EXACT","target":"method:class:example.orders.InventoryClient#reserve(Lexample/orders/ReservationRequest;)Ljava/lang/String;"},
            {"kind":"RETURN"}
        ]);
        let validated = qualified(events, json!([]));
        assert!(validated.source_eligible());
        assert!(matches!(
            validated.projection.steps[0],
            super::ProjectionStep::If(_)
        ));
        assert!(matches!(
            validated.projection.steps[1],
            super::ProjectionStep::MethodReturn(_)
        ));
        assert!(matches!(
            validated.projection.steps[2],
            super::ProjectionStep::End
        ));
        assert!(matches!(
            validated.projection.steps[3],
            super::ProjectionStep::Action { .. }
        ));
        assert!(matches!(
            validated.projection.steps[4],
            super::ProjectionStep::MethodReturn(_)
        ));

        let rendered = super::render_validated(validated, "Checkout").unwrap();
        let guard = rendered
            .tree
            .find("[D] if (!hasPositiveQuantity(request))")
            .unwrap();
        let early_return = rendered.tree.find("  return").unwrap();
        let reserve = rendered.tree.find("InventoryClient#reserve").unwrap();
        let final_return = rendered.tree.rfind("return").unwrap();
        assert!(
            guard < early_return && early_return < reserve && reserve < final_return,
            "{}",
            rendered.tree
        );
        assert!(
            rendered
                .puml
                .contains("if (!hasPositiveQuantity（request）) then (yes)"),
            "{}",
            rendered.puml
        );
        assert!(
            rendered.puml.contains(":return;\nstop\nendif\n"),
            "{}",
            rendered.puml
        );
        assert!(
            rendered.puml.contains(":InventoryClient#reserve;"),
            "{}",
            rendered.puml
        );
        assert_eq!(
            rendered.puml.matches("stop\n").count(),
            2,
            "{}",
            rendered.puml
        );
        assert_eq!(rendered.origin, "flow");
    }

    #[test]
    fn arbitrary_flow_authority_cannot_turn_return_into_stop() {
        let events = json!([
            {"kind":"CALL","target":"method:class:svc.Repo#save()V"},
            {"kind":"RETURN"}
        ]);
        let projection = super::validate_projection(super::project_flow(
            &events,
            &json!({"authority":"SYNTAX","boundaries":[]}),
            "method:class:svc.Worker#run()V",
        ));
        let rendered = super::render_validated(projection, "Unqualified").unwrap();
        gap_only(&rendered, "FLOW_AUTHORITY_NOT_QUALIFIED");
        assert!(!rendered.tree.contains("Repo#save"), "{}", rendered.tree);
    }

    #[test]
    fn malformed_branches_and_truncation_are_visible_gaps_without_repair() {
        for (events, boundaries, expected) in [
            (
                json!([{"kind":"END"},{"kind":"CALL","target":"method:class:svc.Repo#save()V"}]),
                json!([]),
                "MALFORMED_CONTROL:END_WITHOUT_OPEN_IF",
            ),
            (
                json!([{"kind":"ELSE"},{"kind":"CALL","target":"method:class:svc.Repo#save()V"}]),
                json!([]),
                "MALFORMED_CONTROL:ELSE_WITHOUT_OPEN_IF_OR_DUPLICATE_ELSE",
            ),
            (
                json!([{"kind":"IF","condition":"ready"},{"kind":"CALL","target":"method:class:svc.Repo#save()V"}]),
                json!([]),
                "MALFORMED_CONTROL:IF_WITHOUT_END",
            ),
            (
                json!([{"kind":"CALL","target":"method:class:svc.Repo#save()V"}]),
                json!(["DOCUMENTATION_FLOW_BYTE_BUDGET"]),
                "CONTROL_BOUNDARY:DOCUMENTATION_FLOW_BYTE_BUDGET",
            ),
        ] {
            let rendered = super::render_validated(qualified(events, boundaries), "Gap").unwrap();
            gap_only(&rendered, expected);
            assert!(!rendered.tree.contains("Repo#save"), "{}", rendered.tree);
        }
    }

    #[test]
    fn expression_boundary_keeps_source_eligible_but_flow_only_is_a_gap() {
        let events = json!([
            {"kind":"BOUNDARY"},
            {"kind":"RETURN"}
        ]);
        let flow = qualified(events, json!(["SHORT_CIRCUIT_FLOW_REQUIRES_SOURCE_REVIEW"]));
        assert!(flow.source_eligible());
        let rendered = super::render_validated(flow, "Expression").unwrap();
        gap_only(&rendered, "FLOW_EXPRESSION_ORDER_UNQUALIFIED");
        assert!(
            rendered
                .tree
                .contains("SHORT_CIRCUIT_FLOW_REQUIRES_SOURCE_REVIEW"),
            "{}",
            rendered.tree
        );
    }

    #[test]
    fn expression_boundary_does_not_hide_malformed_control_from_source_gate() {
        for (events, expected) in [
            (
                json!([{"kind":"IF","condition":"ready"},{"kind":"RETURN"}]),
                "MALFORMED_CONTROL:IF_WITHOUT_END",
            ),
            (
                json!([{"kind":"END"},{"kind":"RETURN"}]),
                "MALFORMED_CONTROL:END_WITHOUT_OPEN_IF",
            ),
            (
                json!([{"kind":"IF","condition":"ready"},{"kind":"ELSE"},{"kind":"ELSE"},{"kind":"END"}]),
                "MALFORMED_CONTROL:ELSE_WITHOUT_OPEN_IF_OR_DUPLICATE_ELSE",
            ),
        ] {
            let projection =
                qualified(events, json!(["SHORT_CIRCUIT_FLOW_REQUIRES_SOURCE_REVIEW"]));
            assert!(
                !projection.source_eligible(),
                "malformed expression envelope qualified SOURCE"
            );
            let rendered = super::render_validated(projection, "Malformed expression").unwrap();
            gap_only(&rendered, expected);
        }
    }

    #[test]
    fn http_metadata_keeps_framework_client_call_visible_as_an_attempt() {
        let rendered = super::render_validated(
            qualified(
                json!([{
                    "kind":"CALL",
                    "target":"method:class:org.springframework.web.service.invoker.RestClient$ResponseSpec#body(Ljava/lang/Class;)Ljava/lang/Object;",
                    "http":{"boundary":"candidate path only","candidateMethod":"POST","candidatePath":"/reservations"}
                }]),
                json!([]),
            ),
            "Client attempt",
        )
        .unwrap();
        assert!(
            rendered.tree.contains("ResponseSpec#body"),
            "{}",
            rendered.tree
        );
        assert!(
            rendered.puml.contains("ResponseSpec#body"),
            "{}",
            rendered.puml
        );
        assert!(
            rendered.tree.contains("HTTP_BOUNDARY:candidate path only"),
            "{}",
            rendered.tree
        );
        assert!(!rendered.tree.contains("delivered"), "{}", rendered.tree);
    }

    #[test]
    fn else_condition_and_unsupported_loop_are_noncausal_gaps() {
        let conditioned_else = qualified(
            json!([
                {"kind":"IF","condition":"ready"},
                {"kind":"ELSE","condition":"catch IOException"},
                {"kind":"END"}
            ]),
            json!([]),
        );
        let rendered = super::render_validated(conditioned_else, "Else condition").unwrap();
        gap_only(&rendered, "FLOW_ELSE_CONDITION_UNSUPPORTED");

        let loop_projection = qualified(json!([{"kind":"LOOP","condition":"for-each"}]), json!([]));
        let rendered = super::render_validated(loop_projection, "Loop").unwrap();
        gap_only(&rendered, "FLOW_LOOP_UNSUPPORTED_IN_M1");
    }

    #[test]
    fn repository_and_http_call_leaves_remain_attempts_with_boundary_text() {
        let events = json!([
            {"kind":"CALL","target":"method:class:org.springframework.data.repository.CrudRepository#save(Ljava/lang/Object;)Ljava/lang/Object;"},
            {"kind":"CALL","target":"method:class:org.springframework.web.client.RestTemplate#postForObject(Ljava/lang/String;Ljava/lang/Object;Ljava/lang/Class;)Ljava/lang/Object;"},
            {"kind":"CALL","target":"method:class:org.springframework.web.service.invoker.RestClient#retrieve()Lorg/springframework/web/client/RestClient$ResponseSpec;","http":{"boundary":"terminal retrieve operation is version-dependent","candidateMethod":"POST","candidatePath":"/reservations"}},
            {"kind":"RETURN"}
        ]);
        let rendered = super::render_validated(qualified(events, json!([])), "Effects").unwrap();
        for label in [
            "CrudRepository#save",
            "RestTemplate#postForObject",
            "RestClient#retrieve",
        ] {
            assert!(rendered.tree.contains(label), "{}", rendered.tree);
            assert!(rendered.puml.contains(label), "{}", rendered.puml);
        }
        assert!(
            rendered
                .tree
                .contains("HTTP_BOUNDARY:terminal retrieve operation is version-dependent"),
            "{}",
            rendered.tree
        );
        assert!(!rendered.tree.contains("committed"), "{}", rendered.tree);
        assert!(!rendered.tree.contains("delivered"), "{}", rendered.tree);
    }

    #[test]
    fn if_guard_is_retained_fully_but_only_shortened_for_display() {
        let full = format!("guard_{}", "x".repeat(100));
        let events = json!([
            {"kind":"IF","condition":full},
            {"kind":"RETURN"},
            {"kind":"END"}
        ]);
        let validated = qualified(events, json!([]));
        match &validated.projection.steps[0] {
            super::ProjectionStep::If(condition) => assert_eq!(condition, &full),
            other => panic!("expected IF, got {other:?}"),
        }
        let rendered = super::render_validated(validated, "Long guard").unwrap();
        assert!(rendered.tree.contains(&full), "{}", rendered.tree);
        assert!(rendered.puml.contains("..."), "{}", rendered.puml);
    }
}
