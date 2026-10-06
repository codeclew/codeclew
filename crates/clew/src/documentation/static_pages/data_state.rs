//! Opt-in source syntax transformations joined to compiler variable occurrences.
//! No value evaluation, receiver alias analysis, runtime completion or new traversal.
use super::{model::*, source::Parsed};
use crate::{
    canonical::hash_bytes,
    documentation::{
        check::Check,
        digest, invalid,
        model::{Observation, ServiceEvidence, Source},
    },
    error::ClewError,
};
use std::collections::{BTreeMap, BTreeSet};
use tree_sitter::Node;
const SCHEMA: &str = "codeclew-source-data-state/1.0";
const AUTHORITY: &str = "SOURCE_SYNTAX_WITH_COMPILER_VARIABLE_IDENTITY";
const MAX_BYTES: usize = 1024 * 1024; // Existing source expansion ceiling, not an additional traversal allowance.
const MAX_ROWS: usize = 4096;
type Env = BTreeMap<DataStorage, Vec<String>>;
#[derive(Clone, Default)]
struct Path {
    env: Env,
    guards: Vec<DataGuard>,
    completed: Vec<DataCompletion>,
    terminated: bool,
}
fn children(n: Node<'_>) -> Vec<Node<'_>> {
    let mut c = n.walk();
    n.named_children(&mut c).collect()
}
fn active_nodes(root: Node<'_>) -> Vec<Node<'_>> {
    let mut pending = vec![root];
    let mut result = vec![];
    while let Some(n) = pending.pop() {
        if matches!(
            n.kind(),
            "lambda_expression" | "class_body" | "class_declaration"
        ) {
            continue;
        }
        result.push(n);
        pending.extend(children(n).into_iter().rev());
    }
    result
}
fn gap(code: &str, detail: &str) -> Gap {
    Gap {
        code: code.into(),
        detail: detail.into(),
        citation_id: None,
    }
}
fn opaque(syntax: String, reason: &str) -> DataValue {
    DataValue::Opaque {
        syntax,
        reason: reason.into(),
    }
}

/// Coordinate conversion uses retained exact bytes, including CRLF and UTF-8.
fn position(text: &str, offset: usize, first: u64) -> Option<(u64, usize)> {
    if !text.is_char_boundary(offset) {
        return None;
    }
    let bytes = text.as_bytes();
    let mut at = 0;
    let mut line = first;
    let mut start = 0;
    while at < offset {
        if bytes[at] == b'\r' {
            if bytes.get(at + 1) == Some(&b'\n') {
                at += 1;
            }
            line += 1;
            start = at + 1;
        } else if bytes[at] == b'\n' {
            line += 1;
            start = at + 1;
        }
        at += 1;
    }
    Some((line, offset.checked_sub(start)?))
}
fn offset(text: &str, line: u64, column: usize, first: u64) -> Option<usize> {
    let row = usize::try_from(line.checked_sub(first)?).ok()?;
    let start = if row == 0 {
        0
    } else {
        text.match_indices('\n').nth(row - 1)?.0 + 1
    };
    let end = text[start..]
        .find('\n')
        .map(|n| start + n)
        .unwrap_or(text.len());
    let at = start.checked_add(column)?;
    (at <= end && text.is_char_boundary(at)).then_some(at)
}
/// No substring search: exact global line/column to local callable range.
fn site_range(o: &Observation, e: &ServiceEvidence, body: &Source) -> Option<(usize, usize)> {
    if digest(&o.normalized).ok().as_deref() != Some(o.digest.as_str()) {
        return None;
    }
    if o.normalized["schema"] != "codeclew-java-compiler-fact/1.0"
        || o.normalized["resolution"] != "COMPILER_EXACT"
    {
        return None;
    }
    let site = &o.normalized["variableSite"];
    let source = e.sources.get(site["sourceId"].as_str()?)?;
    let origin = usize::try_from(site["sourceByteStart"].as_u64()?).ok()?;
    let limit = usize::try_from(site["sourceByteEnd"].as_u64()?).ok()?;
    let start = usize::try_from(site["byteStart"].as_u64()?)
        .ok()?
        .checked_sub(origin)?;
    let end = usize::try_from(site["byteEnd"].as_u64()?)
        .ok()?
        .checked_sub(origin)?;
    if limit.checked_sub(origin) != Some(source.text.len())
        || source.file != body.file
        || source.service != body.service
        || source.revision != body.revision
        || o.source_ids != [source.id.clone()]
        || site["service"] != e.service
        || site["revision"] != e.revision
        || site["file"] != source.file
        || site["sourceDigest"] != source.text_digest
        || site["evidenceDigest"] != source.evidence_digest
        || source.text_digest != hash_bytes(source.text.as_bytes())
        || site["sourceStatus"] != "SOURCE_RETAINED"
    {
        return None;
    }
    let exact = source.text.get(start..end)?;
    if start >= end || site["spanDigest"] != hash_bytes(exact.as_bytes()) {
        return None;
    }
    let (sl, sc) = position(&source.text, start, source.start_line)?;
    let (el, ec) = position(&source.text, end, source.start_line)?;
    let bs = offset(&body.text, sl, sc, body.start_line)?;
    let be = offset(&body.text, el, ec, body.start_line)?;
    // Callable snippets normalize CRLF. Only exact normalized full spans join.
    let normalized = exact.replace("\r\n", "\n").replace('\r', "\n");
    (body.text.get(bs..be)? == normalized).then_some((bs, be))
}
struct Flow<'a> {
    parsed: &'a Parsed,
    node: &'a SourceCallNode,
    facts: BTreeMap<(usize, usize), &'a Observation>,
    declarations: &'a BTreeMap<String, Observation>,
    source_id: &'a str,
    exhausted: bool,
    construction_bytes: usize,
    result: NodeDataState,
}
impl Flow<'_> {
    fn reserve(&mut self, bytes: usize) -> bool {
        self.construction_bytes = self.construction_bytes.saturating_add(bytes);
        if self.construction_bytes > MAX_BYTES {
            self.exhausted = true;
            false
        } else {
            true
        }
    }
    fn join(&mut self, id: &str, paths: Vec<Path>) -> Path {
        let paths: Vec<_> = paths.into_iter().filter(|p| !p.terminated).collect();
        let Some(first) = paths.first() else {
            return Path {
                terminated: true,
                ..Path::default()
            };
        };
        if paths.len() == 1 {
            return first.clone();
        }
        let mut result = first.clone();
        result
            .guards
            .retain(|g| paths.iter().all(|p| p.guards.contains(g)));
        for p in &paths[1..] {
            for c in &p.completed {
                if !result.completed.contains(c) {
                    result.completed.push(c.clone());
                }
            }
        }
        let storages: BTreeSet<_> = paths.iter().flat_map(|p| p.env.keys().cloned()).collect();
        for storage in storages {
            let alternatives: Vec<_> = paths
                .iter()
                .map(|p| DataGuardedAlternative {
                    definitions: p.env.get(&storage).cloned().unwrap_or_default(),
                    conditions: p.guards.clone(),
                })
                .collect();
            if alternatives
                .iter()
                .all(|a| a.definitions == alternatives[0].definitions)
            {
                continue;
            }
            let key = format!(
                "{id}/choice/{}",
                hash_bytes(&serde_json::to_vec(&storage).unwrap())
            );
            let definition = DataDefinition {
                id: key.clone(),
                storage: Some(storage.clone()),
                value: DataValue::Choice { alternatives },
                conditions: result.guards.clone(),
                normal_completion_of: vec![],
                citation_id: None,
            };
            if !self.reserve(serde_json::to_vec(&definition).unwrap().len()) {
                result.terminated = true;
                return result;
            }
            self.result.definitions.push(definition);
            result.env.insert(storage, vec![key]);
        }
        result
    }
    fn frontier(&mut self, code: &str, detail: &str) {
        if !self.result.gaps.iter().any(|g| g.code == code) {
            self.result.gaps.push(gap(code, detail));
        }
    }
    fn fact(&self, n: Node<'_>) -> Option<&Observation> {
        self.facts.get(&self.parsed.range(n)).copied()
    }
    fn receiver(&self, n: Node<'_>) -> String {
        if n.kind() == "this" || self.parsed.text(n) == "this" {
            return "THIS".into();
        }
        if let Some(o) = self.fact(n) {
            return format!(
                "INPUT:{}",
                o.normalized["variableIdentity"]
                    .as_str()
                    .unwrap_or("unavailable")
            );
        }
        format!("UNRESOLVED:{}", self.parsed.text(n))
    }
    fn storage(&mut self, n: Node<'_>) -> Option<DataStorage> {
        let o = self.fact(n)?;
        let identity = o.normalized["variableIdentity"].as_str()?.to_owned();
        let kind = o.normalized["variableKind"].as_str()?.to_owned();
        let receiver = if kind == "FIELD" {
            let declaration_id = o.normalized["declarationObservationId"]
                .as_str()
                .map(str::to_owned);
            let declaration = declaration_id
                .as_ref()
                .and_then(|id| self.declarations.get(id))
                .filter(|d| {
                    d.kind == "SYMBOL"
                        && d.service == self.node.service
                        && d.normalized["scope"] == self.node.scope
                        && d.normalized["declarationKind"] == "FIELD"
                        && d.symbol == identity
                        && d.normalized["symbolIdentity"] == identity
                });
            let available = declaration.is_some();
            let static_field = declaration.is_some_and(|d| {
                d.normalized["modifiers"]
                    .as_array()
                    .is_some_and(|m| m.iter().any(|v| v == "STATIC"))
            });
            if let Some(d) = declaration
                && !self.result.field_declarations.contains(&d.id)
            {
                self.result.field_declarations.push(d.id.clone());
            }
            if !available {
                self.frontier("FIELD_DECLARATION_UNAVAILABLE","An external variable declaration is unavailable; its storage cannot be unified.");
            }
            Some(if !available {
                "UNRESOLVED_DECLARATION".into()
            } else if static_field {
                "STATIC".into()
            } else if n.kind() == "field_access" {
                n.child_by_field_name("object")
                    .map(|r| self.receiver(r))
                    .unwrap_or_else(|| "UNRESOLVED".into())
            } else {
                "THIS".into()
            })
        } else {
            None
        };
        Some(DataStorage {
            identity,
            kind,
            receiver,
        })
    }
    fn simple_target(n: Node<'_>) -> bool {
        n.kind() == "identifier"
            || (n.kind() == "field_access"
                && n.child_by_field_name("object")
                    .is_some_and(|r| matches!(r.kind(), "identifier" | "this" | "super")))
    }
    fn stop_effects(&mut self, path: &mut Path, code: &str, detail: &str) {
        path.terminated = true;
        self.frontier(code, detail);
    }
    fn writable(storage: &DataStorage) -> bool {
        storage.kind != "FIELD" || matches!(storage.receiver.as_deref(), Some("THIS" | "STATIC"))
    }
    fn read(&mut self, n: Node<'_>, path: &Path) -> DataValue {
        if let Some(storage) = self.storage(n) {
            let references = path
                .env
                .get(&storage)
                .map(|ids| ids.iter().map(String::len).sum::<usize>())
                .unwrap_or(0);
            if !self.reserve(
                references
                    + storage.identity.len()
                    + storage.receiver.as_ref().map(String::len).unwrap_or(0),
            ) {
                return opaque(String::new(), "DATA_STATE_BUDGET");
            }
            DataValue::Read {
                alternatives: if Self::writable(&storage) {
                    path.env.get(&storage).cloned().unwrap_or_default()
                } else {
                    vec![]
                },
                storage,
            }
        } else {
            self.frontier("DATA_SITE_UNAVAILABLE","A variable expression has no unique exact compiler occurrence join; no spelling fallback was used.");
            opaque(self.parsed.text(n), "EXACT_VARIABLE_SITE_UNAVAILABLE")
        }
    }
    fn expression(&mut self, n: Node<'_>, path: &mut Path) -> DataValue {
        if path.terminated {
            return opaque(String::new(), "TRANSFER_ALREADY_WITHHELD");
        }
        if !self.reserve(self.parsed.range(n).1 - self.parsed.range(n).0) {
            path.terminated = true;
            return opaque(String::new(), "DATA_STATE_BUDGET");
        }
        match n.kind() {
            "identifier" | "field_access" => {
                if !Self::simple_target(n) {
                    self.stop_effects(path,"OPAQUE_RECEIVER_EFFECTS","Unsupported field receiver evaluation may have effects; this and following transformations are withheld.");
                    return opaque(self.parsed.text(n), "RECEIVER_EFFECTS_WITHHELD");
                }
                self.read(n, path)
            }
            "string_literal"
            | "character_literal"
            | "decimal_integer_literal"
            | "hex_integer_literal"
            | "decimal_floating_point_literal"
            | "true"
            | "false"
            | "null_literal" => DataValue::Literal {
                text: self.parsed.text(n),
            },
            "parenthesized_expression" => children(n)
                .first()
                .map(|c| self.expression(*c, path))
                .unwrap_or_else(|| opaque(self.parsed.text(n), "EMPTY_EXPRESSION")),
            "unary_expression" => {
                if let Some(operand) = n.child_by_field_name("operand") {
                    let value = self.expression(operand, path);
                    if path.terminated {
                        return opaque(String::new(), "CHILD_EFFECTS_WITHHELD");
                    }
                    DataValue::Unary {
                        operator: n
                            .child_by_field_name("operator")
                            .map(|o| self.parsed.text(o))
                            .unwrap_or_default(),
                        operand: Box::new(value),
                    }
                } else {
                    opaque(self.parsed.text(n), "UNARY_SHAPE")
                }
            }
            "binary_expression" => {
                let op = n
                    .child_by_field_name("operator")
                    .map(|n| self.parsed.text(n))
                    .unwrap_or_default();
                if matches!(op.as_str(), "&&" | "||") {
                    path.terminated = true;
                    self.frontier(
                        "OPAQUE_SHORT_CIRCUIT",
                        "Short-circuit value/control effects are not transferred.",
                    );
                    return opaque(self.parsed.text(n), "SHORT_CIRCUIT");
                }
                match (
                    n.child_by_field_name("left"),
                    n.child_by_field_name("right"),
                ) {
                    (Some(l), Some(r)) => {
                        let left = self.expression(l, path);
                        if path.terminated {
                            return opaque(String::new(), "CHILD_EFFECTS_WITHHELD");
                        }
                        let right = self.expression(r, path);
                        if path.terminated {
                            return opaque(String::new(), "CHILD_EFFECTS_WITHHELD");
                        }
                        DataValue::Binary {
                            operator: op,
                            left: Box::new(left),
                            right: Box::new(right),
                        }
                    }
                    _ => opaque(self.parsed.text(n), "BINARY_SHAPE"),
                }
            }
            "method_invocation" | "object_creation_expression" => {
                let range = self.parsed.range(n);
                let edge = self.node.calls.iter().find(|e| {
                    e.call.as_ref().is_some_and(|call| {
                        self.node.citations.get(&call.citation_id).is_some_and(|c| {
                            c.source_id == self.source_id && (c.start_byte, c.end_byte) == range
                        })
                    })
                });
                let Some(edge) = edge else {
                    self.frontier("CALL_OCCURRENCE_UNAVAILABLE","No unique retained call occurrence is available; result and effects remain opaque.");
                    path.terminated = true;
                    return opaque(self.parsed.text(n), "CALL_OCCURRENCE_UNAVAILABLE");
                };
                let occurrence = edge
                    .occurrence_path
                    .as_ref()
                    .expect("Java data-state edges have occurrence paths")
                    .clone();
                let target = edge.target_node.clone();
                let status = edge.status.clone();
                let receiver = n
                    .child_by_field_name("object")
                    .map(|r| Box::new(self.expression(r, path)));
                if path.terminated {
                    return opaque(String::new(), "RECEIVER_EFFECTS_WITHHELD");
                }
                let mut arguments = Vec::new();
                for argument in n
                    .child_by_field_name("arguments")
                    .map(children)
                    .unwrap_or_default()
                {
                    let value = self.expression(argument, path);
                    if path.terminated {
                        return opaque(String::new(), "ARGUMENT_EFFECTS_WITHHELD");
                    }
                    arguments.push(value);
                }
                if !self.reserve(serde_json::to_vec(&arguments).unwrap().len()) {
                    path.terminated = true;
                    return opaque(String::new(), "DATA_STATE_BUDGET");
                }
                self.result.calls.push(DataCall {
                    mapping_authority: "DECLARED_TARGET_SOURCE_CONDITIONAL".into(),
                    conditions: path.guards.clone(),
                    occurrence: occurrence.clone(),
                    target_node: target.clone(),
                    arguments: arguments
                        .iter()
                        .cloned()
                        .enumerate()
                        .map(|(slot, value)| DataArgument {
                            slot,
                            value,
                            formal_identity: None,
                        })
                        .collect(),
                    return_definitions: vec![],
                    normal_completion_of: path.completed.clone(),
                });
                path.completed.push(DataCompletion {
                    occurrence: occurrence.clone(),
                    conditions: path.guards.clone(),
                });
                // Any call can change fields through receivers, aliases or callbacks.
                // Locals/formals keep their Java bindings; no receiver solver is implied.
                let fields: Vec<_> = path
                    .env
                    .keys()
                    .filter(|s| s.kind == "FIELD")
                    .cloned()
                    .collect();
                for storage in fields {
                    let prior_definitions = path.env.get(&storage).cloned().unwrap_or_default();
                    self.frontier("CALL_FIELD_INTERFERENCE","Calls can change field state through aliases or callbacks; preceding field definitions do not exhaust the post-call value.");
                    self.define(
                        format!(
                            "after/{occurrence}/{}",
                            hash_bytes(&serde_json::to_vec(&storage).unwrap())
                        ),
                        Some(storage),
                        DataValue::Interference {
                            occurrence: occurrence.clone(),
                            prior_definitions,
                        },
                        path,
                        n,
                    );
                }
                DataValue::CallResult {
                    occurrence,
                    receiver,
                    arguments,
                    target_node: target,
                    target_authority: "DECLARED_TARGET_SOURCE_CONDITIONAL".into(),
                    frontier: (status != "RETAINED_DECLARED_BODY").then_some(status),
                }
            }
            _ => {
                self.frontier("OPAQUE_EXPRESSION","Unsupported expressions retain literal syntax; their transformations/effects are not evaluated.");
                path.terminated = true;
                opaque(self.parsed.text(n), "UNSUPPORTED_EXPRESSION")
            }
        }
    }
    fn define(
        &mut self,
        id: String,
        storage: Option<DataStorage>,
        value: DataValue,
        path: &mut Path,
        n: Node<'_>,
    ) {
        if path.terminated {
            return;
        }
        let id = format!(
            "{id}/guard-{}",
            hash_bytes(&serde_json::to_vec(&path.guards).unwrap())[7..23].to_owned()
        );
        if let Some(s) = &storage {
            if Self::writable(s) {
                path.env.insert(s.clone(), vec![id.clone()]);
            } else {
                self.frontier("RECEIVER_STORAGE_OPAQUE","Writes through non-this receivers do not establish source-local instance storage or aliases.");
            }
        }
        let range = self.parsed.range(n);
        let citation_id = self
            .node
            .citations
            .values()
            .find(|c| {
                c.source_id == self.source_id && c.start_byte <= range.0 && c.end_byte >= range.1
            })
            .map(|c| c.id.clone());
        let definition = DataDefinition {
            id,
            storage,
            value,
            conditions: path.guards.clone(),
            normal_completion_of: path.completed.clone(),
            citation_id,
        };
        if self.reserve(serde_json::to_vec(&definition).unwrap().len()) {
            self.result.definitions.push(definition);
        }
    }
    fn statement(&mut self, n: Node<'_>, id: &str, mut path: Path) -> Vec<Path> {
        if path.terminated {
            return vec![path];
        }
        if self.result.definitions.len() + self.result.calls.len() >= MAX_ROWS {
            self.exhausted = true;
            path.terminated = true;
            return vec![path];
        }
        match n.kind() {
            "block" => {
                let mut paths = vec![path];
                for (i, c) in children(n).into_iter().enumerate() {
                    if paths.len() > MAX_ROWS {
                        self.exhausted = true;
                        return vec![];
                    }
                    paths = paths
                        .into_iter()
                        .flat_map(|p| {
                            if p.terminated {
                                vec![p]
                            } else {
                                self.statement(c, &format!("{id}/{i}"), p)
                            }
                        })
                        .collect();
                }
                paths
            }
            "if_statement" => {
                let Some(condition) = n.child_by_field_name("condition") else {
                    return vec![path];
                };
                let expression = self.parsed.text(condition); // Guard is source syntax, not evaluated.
                let _ = self.expression(condition, &mut path);
                if path.terminated {
                    return vec![path];
                }
                let mut yes = path.clone();
                yes.guards.push(DataGuard {
                    expression: expression.clone(),
                    holds: true,
                });
                let mut no = path;
                no.guards.push(DataGuard {
                    expression,
                    holds: false,
                });
                let mut rows = n
                    .child_by_field_name("consequence")
                    .map(|c| self.statement(c, &format!("{id}/true"), yes.clone()))
                    .unwrap_or_else(|| vec![yes]);
                rows.extend(
                    n.child_by_field_name("alternative")
                        .map(|c| self.statement(c, &format!("{id}/false"), no.clone()))
                        .unwrap_or_else(|| vec![no]),
                );
                vec![self.join(id, rows)]
            }
            "local_variable_declaration" => {
                let vars = children(n)
                    .into_iter()
                    .filter(|c| c.kind() == "variable_declarator")
                    .collect::<Vec<_>>();
                if vars.len() != 1 {
                    self.frontier(
                        "OPAQUE_MULTI_DECLARATION",
                        "Multiple declarators do not have an unambiguous declaration-span join; initializer effects and following transformations are withheld.",
                    );
                    path.terminated = true;
                    return vec![path];
                }
                let storage = self.storage(n).or_else(|| self.storage(vars[0]));
                if let Some(s) = storage {
                    let value = vars[0]
                        .child_by_field_name("value")
                        .map(|v| self.expression(v, &mut path))
                        .unwrap_or_else(|| opaque(self.parsed.text(n), "UNINITIALIZED"));
                    self.define(id.into(), Some(s), value, &mut path, n);
                } else {
                    self.frontier(
                        "DATA_SITE_UNAVAILABLE",
                        "Local declaration lacks an exact compiler span join; initializer effects and following transformations are withheld.",
                    );
                    path.terminated = true;
                }
                vec![path]
            }
            "expression_statement" => {
                if let Some(expr) = children(n).first().copied() {
                    if expr.kind() == "assignment_expression" {
                        if let (Some(left), Some(right)) = (
                            expr.child_by_field_name("left"),
                            expr.child_by_field_name("right"),
                        ) {
                            if !Self::simple_target(left) {
                                self.stop_effects(&mut path,"OPAQUE_ASSIGNMENT_TARGET_EFFECTS","Unsupported assignment receiver or array-index evaluation may have effects; this and following transformations are withheld.");
                                return vec![path];
                            }
                            let storage = self.storage(left);
                            if storage.is_none() {
                                self.stop_effects(&mut path,"DATA_SITE_UNAVAILABLE","Assignment target lacks an exact compiler span join; its write and following transformations are withheld.");
                                return vec![path];
                            }
                            let prior = self.read(left, &path);
                            let right = self.expression(right, &mut path);
                            let op = expr
                                .child_by_field_name("operator")
                                .map(|n| self.parsed.text(n))
                                .unwrap_or_else(|| "=".into());
                            let value = if op == "=" {
                                right
                            } else {
                                DataValue::Binary {
                                    operator: op.trim_end_matches('=').into(),
                                    left: Box::new(prior),
                                    right: Box::new(right),
                                }
                            };
                            if storage.is_some() {
                                self.define(id.into(), storage, value, &mut path, n);
                            } else {
                                self.frontier(
                                    "DATA_SITE_UNAVAILABLE",
                                    "Assignment target lacks an exact compiler span join.",
                                );
                            }
                        }
                    } else {
                        let _ = self.expression(expr, &mut path);
                    }
                }
                vec![path]
            }
            "return_statement" | "throw_statement" => {
                let value = children(n)
                    .first()
                    .map(|v| self.expression(*v, &mut path))
                    .unwrap_or_else(|| opaque(String::new(), "VOID_RETURN"));
                if path.terminated {
                    return vec![path];
                }
                if n.kind() == "return_statement" {
                    self.define(format!("{id}/return"), None, value, &mut path, n);
                }
                path.terminated = true;
                vec![path]
            }
            _ => {
                self.frontier("OPAQUE_CONTROL","Loops, deferred bodies and unsupported controls are not transferred. Following transformations are withheld.");
                path.terminated = true;
                vec![path]
            }
        }
    }
}
fn project(e: &ServiceEvidence, node: &SourceCallNode) -> Result<NodeDataState, ClewError> {
    let declaration = &e.observations[&node.callable.declaration_id];
    let source = declaration
        .source_ids
        .iter()
        .filter_map(|id| e.sources.get(id))
        .find(|s| {
            Parsed::new(&s.text)
                .and_then(|p| p.callable(declaration).map(|_| ()))
                .is_some()
        });
    let mut result = NodeDataState {
        schema: SCHEMA.into(),
        authority: AUTHORITY.into(),
        data_state_digest: String::new(),
        definitions: vec![],
        calls: vec![],
        field_declarations: vec![],
        gaps: vec![],
    };
    let Some(source) = source else {
        result.gaps.push(gap(
            "DATA_BODY_UNAVAILABLE",
            "The retained callable body cannot be parsed.",
        ));
        return Ok(result);
    };
    let parsed = Parsed::new(&source.text)
        .ok_or_else(|| invalid("data-state callable parser unavailable"))?;
    let callable = parsed
        .callable(declaration)
        .ok_or_else(|| invalid("data-state callable is ambiguous"))?;
    if callable.has_error() {
        result.gaps.push(gap(
            "DATA_BODY_PARTIAL",
            "The retained callable has syntax errors; no data transformation is promoted.",
        ));
        return Ok(result);
    }
    let mut facts = BTreeMap::new();
    let mut bytes = 0;
    let mut missing = false;
    for o in e.observations.values().filter(|o| {
        matches!(o.kind.as_str(), "VARIABLE_DECLARATION" | "VARIABLE_ACCESS")
            && o.normalized["callableObservationId"] == node.callable.declaration_id
            && o.normalized["scope"] == node.scope
    }) {
        bytes += serde_json::to_vec(o)
            .map_err(|_| invalid("data-state fact encoding failed"))?
            .len();
        if bytes > MAX_BYTES || facts.len() >= MAX_ROWS {
            return Err(invalid(
                "expandDataState exceeds bounded fact budget; narrow the selection",
            ));
        }
        if let Some(range) = site_range(o, e, source) {
            if facts.insert(range, o).is_some() {
                return Err(invalid("data-state variable site is ambiguous"));
            }
        } else {
            missing = true;
        }
    }
    if missing {
        result.gaps.push(gap(
            "DATA_SITE_UNAVAILABLE",
            "Old or inconsistent variable sites cannot supply exact column identities.",
        ));
    }
    let mut flow = Flow {
        parsed: &parsed,
        node,
        facts,
        declarations: &e.observations,
        source_id: &source.id,
        exhausted: false,
        construction_bytes: 0,
        result,
    };
    let mut path = Path::default();
    // Formal definitions are immutable per cached callable, never caller-specific.
    for o in flow
        .facts
        .values()
        .filter(|o| o.kind == "VARIABLE_DECLARATION" && o.normalized["variableKind"] == "PARAMETER")
    {
        let storage = DataStorage {
            identity: o.normalized["variableIdentity"]
                .as_str()
                .unwrap_or("")
                .into(),
            kind: "PARAMETER".into(),
            receiver: None,
        };
        let id = format!("formal/{}", storage.identity);
        path.env.insert(storage.clone(), vec![id.clone()]);
        flow.result.definitions.push(DataDefinition {
            id,
            storage: Some(storage.clone()),
            value: DataValue::Input { storage },
            conditions: vec![],
            normal_completion_of: vec![],
            citation_id: None,
        });
    }
    if let Some(body) = callable.child_by_field_name("body") {
        // Source-local fields start with an explicit opaque incoming value. This
        // keeps the old value as an alternative when only one branch writes.
        for n in active_nodes(body)
            .into_iter()
            .filter(|n| matches!(n.kind(), "identifier" | "field_access"))
        {
            if !flow
                .fact(n)
                .is_some_and(|o| o.normalized["variableKind"] == "FIELD")
            {
                continue;
            }
            let Some(storage) = flow.storage(n).filter(Flow::writable) else {
                continue;
            };
            if path.env.contains_key(&storage) {
                continue;
            }
            let id = format!(
                "incoming/{}",
                hash_bytes(&serde_json::to_vec(&storage).unwrap())
            );
            let definition = DataDefinition {
                id: id.clone(),
                storage: Some(storage.clone()),
                value: DataValue::Input {
                    storage: storage.clone(),
                },
                conditions: vec![],
                normal_completion_of: vec![],
                citation_id: None,
            };
            if !flow.reserve(serde_json::to_vec(&definition).unwrap().len()) {
                return Err(invalid(
                    "expandDataState exceeds incoming storage budget; narrow the selection",
                ));
            }
            path.env.insert(storage, vec![id]);
            flow.result.definitions.push(definition);
        }
        let _ = flow.statement(body, "body", path);
    }
    if flow.exhausted {
        return Err(invalid(
            "expandDataState exceeds bounded path/definition budget; narrow the selection",
        ));
    }
    flow.result.field_declarations.sort();
    flow.result.field_declarations.dedup();
    Ok(flow.result)
}
fn semantic_digest(state: &NodeDataState) -> Result<String, ClewError> {
    let mut semantic = state.clone();
    semantic.data_state_digest.clear();
    semantic.field_declarations.clear();
    for d in &mut semantic.definitions {
        d.citation_id = None;
    }
    digest(&semantic)
}
pub(super) fn attach(checked: &Check, p: &mut BundleProjection) -> Result<(), ClewError> {
    if !p.pages.iter().any(|page| page.selection.expand_data_state) {
        return Ok(());
    }
    let graph = p
        .source_call_graph
        .as_mut()
        .ok_or_else(|| invalid("expandDataState requires source-call graph"))?;
    let selected: BTreeSet<_> = p
        .pages
        .iter()
        .filter(|p| p.selection.expand_data_state)
        .flat_map(|p| {
            p.examined_sources
                .iter()
                .flat_map(|s| s.memberships.iter().map(|m| m.node.clone()))
        })
        .collect();
    attach_graph(checked, graph, &selected)?;
    for page in p.pages.iter_mut().filter(|p| p.selection.expand_data_state) {
        let nodes: Vec<_> = page
            .examined_sources
            .iter()
            .flat_map(|e| e.memberships.iter().map(|m| m.node.clone()))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let hashes: Vec<_> = nodes
            .iter()
            .map(|id| {
                (
                    id,
                    graph.nodes[id]
                        .data_state
                        .as_ref()
                        .map(|s| &s.data_state_digest),
                )
            })
            .collect();
        page.data_state = Some(ExaminedDataState {
            schema: SCHEMA.into(),
            authority: AUTHORITY.into(),
            data_state_digest: digest(&hashes)?,
            nodes,
        });
    }
    Ok(())
}

/// Shared graph consumer: same fact, source, row and IR ceilings as native pages.
pub(super) fn attach_graph(
    checked: &Check,
    graph: &mut SourceCallGraph,
    selected: &BTreeSet<String>,
) -> Result<(), ClewError> {
    let mut bytes = 0;
    let mut rows = 0;
    let mut fact_bytes = 0;
    let mut fact_count = 0;
    let mut retained_sources = BTreeSet::new();
    for id in selected {
        let node = &graph.nodes[id];
        let e = &checked.services[&node.service];
        for o in e.observations.values().filter(|o| {
            matches!(o.kind.as_str(), "VARIABLE_DECLARATION" | "VARIABLE_ACCESS")
                && o.normalized["callableObservationId"] == node.callable.declaration_id
                && o.normalized["scope"] == node.scope
        }) {
            fact_count += 1;
            fact_bytes += serde_json::to_vec(o)
                .map_err(|_| invalid("data-state fact encoding failed"))?
                .len();
            for source in &o.source_ids {
                if retained_sources.insert(source.clone()) {
                    fact_bytes += e.sources.get(source).map(|s| s.text.len()).unwrap_or(0);
                }
            }
            if fact_count > MAX_ROWS || fact_bytes > MAX_BYTES {
                return Err(invalid(
                    "expandDataState exceeds cumulative retained-fact budget; narrow the selection",
                ));
            }
        }
    }
    for id in selected {
        let node = graph
            .nodes
            .get(id)
            .ok_or_else(|| invalid("examined data-state node is unavailable"))?;
        let state = project(&checked.services[&node.service], node)?;
        rows += state.definitions.len() + state.calls.len();
        bytes += serde_json::to_vec(&state)
            .map_err(|_| invalid("data-state encoding failed"))?
            .len();
        if bytes > MAX_BYTES || rows > MAX_ROWS {
            return Err(invalid(
                "expandDataState exceeds bounded output budget; narrow the selection",
            ));
        }
        let node = graph.nodes.get_mut(id).unwrap();
        let e = &checked.services[&node.service];
        for o in e.observations.values().filter(|o| {
            matches!(o.kind.as_str(), "VARIABLE_DECLARATION" | "VARIABLE_ACCESS")
                && o.normalized["callableObservationId"] == node.callable.declaration_id
                && o.normalized["scope"] == node.scope
        }) {
            node.observations.insert(o.id.clone(), o.clone());
            for id in &o.source_ids {
                if let Some(source) = e.sources.get(id) {
                    node.sources.insert(id.clone(), source.clone());
                }
            }
        }
        node.data_state = Some(state);
    }
    // Return links belong to each call occurrence; cached callee inputs stay intact.
    let returns: BTreeMap<_, Vec<_>> = graph
        .nodes
        .iter()
        .filter_map(|(id, n)| {
            n.data_state.as_ref().map(|s| {
                (
                    id.clone(),
                    s.definitions
                        .iter()
                        .filter(|d| d.storage.is_none())
                        .map(|d| d.id.clone())
                        .collect(),
                )
            })
        })
        .collect();
    let formals: BTreeMap<_, _> = graph
        .nodes
        .iter()
        .flat_map(|(id, n)| {
            n.data_state.iter().flat_map(move |s| {
                s.definitions.iter().filter_map(move |d| {
                    let DataValue::Input { storage } = &d.value else {
                        return None;
                    };
                    let slot = storage
                        .identity
                        .rsplit_once("/slot/")?
                        .1
                        .parse::<usize>()
                        .ok()?;
                    Some(((id.clone(), slot), storage.identity.clone()))
                })
            })
        })
        .collect();
    for node in graph.nodes.values_mut() {
        if let Some(state) = &mut node.data_state {
            for call in &mut state.calls {
                if let Some(target) = &call.target_node {
                    call.return_definitions = returns.get(target).cloned().unwrap_or_default();
                    for argument in &mut call.arguments {
                        argument.formal_identity =
                            formals.get(&(target.clone(), argument.slot)).cloned();
                    }
                }
            }
            state.data_state_digest = semantic_digest(state)?;
            for field in &state.field_declarations {
                graph
                    .reverse_field_references
                    .entry(field.clone())
                    .or_default()
                    .push(node.id.clone());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn source(text: &str) -> Source {
        Source {
            id: "body".into(),
            service: "fixture".into(),
            revision: "a".repeat(40),
            file: "Fixture.java".into(),
            start_line: 1,
            end_line: text.lines().count() as u64,
            text: text.into(),
            text_digest: hash_bytes(text.as_bytes()),
            evidence_digest: "synthetic-receipt".into(),
            authority: "EXACT_SNAPSHOT_TEXT".into(),
            occurrence: None,
            url: None,
        }
    }
    fn evidence(text: &str) -> ServiceEvidence {
        let normalized = json!({"name":"prepare","declarationKind":"METHOD","scope":"scope"});
        let o = Observation {
            id: "method".into(),
            kind: "SYMBOL".into(),
            service: "fixture".into(),
            symbol: "method:Fixture#prepare()".into(),
            digest: digest(&normalized).unwrap(),
            normalized,
            source_ids: vec!["body".into()],
        };
        ServiceEvidence {
            schema: "synthetic-unit-fixture".into(),
            service: "fixture".into(),
            revision: "a".repeat(40),
            service_digest: "service".into(),
            extractor: "synthetic".into(),
            runtime_mode: "STATIC".into(),
            coverage: "SEMANTIC".into(),
            boundaries: vec![],
            entrypoints: vec![],
            observations: BTreeMap::from([("method".into(), o)]),
            sources: BTreeMap::from([("body".into(), source(text))]),
            contracts: BTreeMap::new(),
        }
    }
    fn fact(
        e: &mut ServiceEvidence,
        start: usize,
        end: usize,
        identity: &str,
        kind: &str,
        declaration: bool,
    ) {
        let s = &e.sources["body"];
        let id = format!("fact-{start}-{end}");
        let normalized = json!({"schema":"codeclew-java-compiler-fact/1.0","resolution":"COMPILER_EXACT","variableIdentity":identity,"variableKind":kind,"scope":"scope","callableObservationId":"method","declarationObservationId":format!("declaration-{identity}"),"variableSite":{"sourceId":"body","service":"fixture","revision":e.revision,"file":s.file,"sourceDigest":s.text_digest,"evidenceDigest":s.evidence_digest,"sourceStatus":"SOURCE_RETAINED","sourceByteStart":0,"sourceByteEnd":s.text.len(),"byteStart":start,"byteEnd":end,"spanDigest":hash_bytes(&s.text.as_bytes()[start..end])}});
        e.observations.insert(
            id.clone(),
            Observation {
                id,
                kind: if declaration {
                    "VARIABLE_DECLARATION"
                } else {
                    "VARIABLE_ACCESS"
                }
                .into(),
                service: "fixture".into(),
                symbol: "method:Fixture#prepare()".into(),
                digest: digest(&normalized).unwrap(),
                normalized,
                source_ids: vec!["body".into()],
            },
        );
    }
    fn nodes<'a>(root: Node<'a>) -> Vec<Node<'a>> {
        let mut result = vec![root];
        for c in children(root) {
            result.extend(nodes(c));
        }
        result
    }
    fn node() -> SourceCallNode {
        SourceCallNode {
            id: "node".into(),
            service: "fixture".into(),
            scope: "scope".into(),
            callable: CallableProjection {
                declaration_id: "method".into(),
                symbol: "method:Fixture#prepare()".into(),
                authority: "COMPILER_EXACT".into(),
                citation_id: None,
                control_flow: None,
                source_outline: None,
                retained_call_sites: None,
                steps: vec![],
                state: vec![],
                gaps: vec![],
            },
            calls: vec![],
            citations: BTreeMap::new(),
            observations: BTreeMap::new(),
            sources: BTreeMap::new(),
            examined_source_digest: "old".into(),
            node_projection_kind: None,
            data_state: None,
        }
    }
    fn call_node(e: &ServiceEvidence, parsed: &Parsed) -> SourceCallNode {
        let mut result = node();
        let callable = parsed.callable(&e.observations["method"]).unwrap();
        for (i, n) in nodes(callable)
            .into_iter()
            .filter(|n| n.kind() == "method_invocation")
            .enumerate()
        {
            let (start, end) = parsed.range(n);
            let id = format!("call-citation-{i}");
            let s = &e.sources["body"];
            result.citations.insert(
                id.clone(),
                Citation {
                    id: id.clone(),
                    source_id: "body".into(),
                    service: "fixture".into(),
                    revision: e.revision.clone(),
                    file: s.file.clone(),
                    start_line: 1,
                    end_line: 1,
                    start_byte: start,
                    end_byte: end,
                    text_digest: hash_bytes(&s.text.as_bytes()[start..end]),
                    evidence_digest: s.evidence_digest.clone(),
                    authority: "SYNTHETIC_UNIT_BINDING".into(),
                    url: None,
                },
            );
            result.calls.push(serde_json::from_value(json!({"occurrencePath":format!("body/call/{i}"),"statementId":"s","sourceIdentity":"method","targetScope":"scope","callSourceIds":[],"relationDigest":null,"call":{"expression":parsed.text(n),"receiver":null,"name":"f","arguments":[],"target":null,"authority":"SYNTHETIC_UNIT_BINDING","relationId":null,"citationId":id,"phase":"SOURCE_CALL","externalBoundary":null,"gaps":[]},"conditions":[],"reachable":true,"targetDeclaration":null,"targetNode":null,"status":"BODY_UNAVAILABLE","receiverLineage":"UNRESOLVED","runtimeDispatch":"UNRESOLVED","frontiers":[]})).unwrap());
        }
        result
    }
    fn field_facts(e: &mut ServiceEvidence, parsed: &Parsed) {
        let normalized = json!({"modifiers":[],"declarationKind":"FIELD","scope":"scope","symbolIdentity":"field:name"});
        e.observations.insert(
            "declaration-field:name".into(),
            Observation {
                id: "declaration-field:name".into(),
                kind: "SYMBOL".into(),
                service: "fixture".into(),
                symbol: "field:name".into(),
                digest: digest(&normalized).unwrap(),
                normalized,
                source_ids: vec![],
            },
        );
        for n in nodes(parsed.callable(&e.observations["method"]).unwrap())
            .into_iter()
            .filter(|n| n.kind() == "field_access")
        {
            let (s, t) = parsed.range(n);
            fact(e, s, t, "field:name", "FIELD", false);
        }
    }
    // Unit compiler observations are explicitly synthetic and bound to AST ranges;
    // the ignored native journey below qualifies actual Trees-produced identities.
    fn local_facts(e: &mut ServiceEvidence, parsed: &Parsed) {
        let callable = parsed.callable(&e.observations["method"]).unwrap();
        for n in nodes(callable) {
            let (start, end) = parsed.range(n);
            match n.kind() {
                "formal_parameter" => {
                    let name = n.child_by_field_name("name").unwrap();
                    fact(
                        e,
                        start,
                        end,
                        &format!("parameter:{}", parsed.text(name)),
                        "PARAMETER",
                        true,
                    );
                }
                "local_variable_declaration" => {
                    let v = children(n)
                        .into_iter()
                        .find(|n| n.kind() == "variable_declarator")
                        .unwrap();
                    let name = parsed.text(v.child_by_field_name("name").unwrap());
                    fact(
                        e,
                        start,
                        end,
                        &format!("local:{name}"),
                        "LOCAL_VARIABLE",
                        true,
                    );
                }
                "identifier" => {
                    let name = parsed.text(n);
                    if matches!(
                        name.as_str(),
                        "chosen" | "transformed" | "value" | "request"
                    ) {
                        fact(
                            e,
                            start,
                            end,
                            &format!("local:{name}"),
                            "LOCAL_VARIABLE",
                            false,
                        );
                    } else if matches!(name.as_str(), "task" | "other") {
                        fact(
                            e,
                            start,
                            end,
                            &format!("parameter:{name}"),
                            "PARAMETER",
                            false,
                        );
                    }
                }
                _ => {}
            }
        }
    }
    fn bridge_work(text: &str) -> crate::documentation::work::Work {
        let mut e = evidence(text);
        let parsed = Parsed::new(text).unwrap();
        local_facts(&mut e, &parsed);
        let o = e.observations.get_mut("method").unwrap();
        o.normalized["schema"] = json!("codeclew-java-compiler-fact/1.0");
        o.normalized["kind"] = json!("DECLARATION");
        o.normalized["symbolIdentity"] = json!(o.symbol);
        o.normalized["ownerIdentity"] = json!("class:Fixture");
        o.normalized["resolution"] = json!("COMPILER_EXACT");
        o.digest = digest(&o.normalized).unwrap();
        let mut work = crate::documentation::work::api_contract_tests::endpoint_context_fixture();
        work.subject = "service:fixture".into();
        work.request.context_profile = Some("process-graph-v1".into());
        work.request.root_declaration = Some("method".into());
        work.request.entrypoint = None;
        work.request.source_data_context = true;
        work.snapshot = Some("synthetic-immutable-snapshot".into());
        work.checked.dependencies = e.observations.clone();
        work.checked.services = BTreeMap::from([("fixture".into(), e)]);
        work.influence = work
            .checked
            .dependencies
            .iter()
            .map(|(id, o)| (id.clone(), o.digest.clone()))
            .collect();
        work.handles = work
            .checked
            .dependencies
            .keys()
            .enumerate()
            .map(|(i, id)| {
                (
                    format!("d{}", i + 1),
                    crate::documentation::work::Handle {
                        kind: "DEPENDENCY".into(),
                        id: id.clone(),
                    },
                )
            })
            .collect();
        work.handles.insert(
            "s1".into(),
            crate::documentation::work::Handle {
                kind: "SOURCE".into(),
                id: "body".into(),
            },
        );
        work
    }
    #[test]
    fn source_data_packet_bridge_binds_guarded_ir_exact_sources_and_default_identity() {
        use crate::documentation::source_data_context as bridge;
        let mut work = bridge_work(
            "String prepare(Task task) { String chosen = task; if (chosen == null) { chosen = \"anonymous\"; } String transformed = chosen.trim(); return \"prefix:\" + transformed; }",
        );
        let projection = bridge::build(&work).unwrap().unwrap();
        let state = &projection.context["nodes"][0]["dataState"];
        let encoded = state.to_string();
        assert!(encoded.contains("CHOICE"), "{encoded}");
        assert!(encoded.contains("CALL_RESULT"), "{encoded}");
        assert!(encoded.contains("DECLARED_TARGET_SOURCE_CONDITIONAL"));
        assert!(encoded.contains("BINARY"));
        assert_eq!(projection.context["runtimeStatus"], "UNKNOWN");
        assert_eq!(
            projection.context["sources"][0]["text"],
            work.checked.services["fixture"].sources["body"].text
        );
        assert!(
            !projection
                .rows
                .iter()
                .any(|r| r["kind"] == "VARIABLE_ACCESS")
        ); // rows retain the real DEPENDENCY kind
        assert!(
            projection
                .rows
                .iter()
                .any(|r| r["record"]["kind"] == "VARIABLE_ACCESS")
        );
        let packet =
            json!({"sourceDataContext":projection.context,"citations":projection.citations});
        bridge::validate_saved(&work, &packet).unwrap();
        assert_eq!(
            packet["sourceDataContext"],
            bridge::build(&work).unwrap().unwrap().context
        );
        for pointer in [
            "/sourceDataContext/sourceDataDigest",
            "/sourceDataContext/snapshot",
            "/sourceDataContext/nodes/0/dataState/authority",
            "/sourceDataContext/sources/0/text",
        ] {
            let mut forged = packet.clone();
            *forged.pointer_mut(pointer).unwrap() = json!("forged");
            assert!(bridge::validate_saved(&work, &forged).is_err(), "{pointer}");
        }
        assert!(bridge::validate_saved(&work, &json!({})).is_err());
        work.request.source_data_context = false;
        assert!(bridge::build(&work).unwrap().is_none());
        assert!(
            serde_json::to_value(&work.request)
                .unwrap()
                .get("sourceDataContext")
                .is_none()
        );
        bridge::validate_saved(&work, &json!({})).unwrap();
        assert!(bridge::validate_saved(&work, &packet).is_err());
        let absent = serde_json::to_value(&work.request).unwrap();
        let mut explicit = absent.clone();
        explicit["sourceDataContext"] = json!(false);
        assert_eq!(
            crate::documentation::bytes(
                &serde_json::from_value::<crate::documentation::work::Request>(absent).unwrap()
            )
            .unwrap(),
            crate::documentation::bytes(
                &serde_json::from_value::<crate::documentation::work::Request>(explicit).unwrap()
            )
            .unwrap()
        );
    }

    #[test]
    fn source_data_context_matches_the_preboundary_canonical_baseline() {
        use crate::documentation::source_data_context as bridge;
        let work = bridge_work(
            "String prepare(Task task) { String chosen = task; if (chosen == null) { chosen = \"anonymous\"; } String transformed = chosen.trim(); return \"prefix:\" + transformed; }",
        );
        let projection = bridge::build(&work).unwrap().unwrap();
        let encoded = crate::documentation::bytes(&projection.context).unwrap();
        assert_eq!(
            crate::canonical::hash_bytes(&encoded),
            "sha256:b710fa2c26f6329e5ecd6070c6884174cd60bb0534ba3ef54484e0c613e39fb8"
        );
    }
    #[test]
    fn source_data_packet_bridge_refuses_missing_provenance_and_cap_before_dispatch() {
        use crate::documentation::source_data_context as bridge;
        let work =
            bridge_work("String prepare(Task task) { String chosen = task; return chosen; }");
        let mut missing = work.clone();
        missing.snapshot = None;
        assert!(bridge::build(&missing).is_err());
        missing = work.clone();
        missing.handles.remove("s1");
        assert!(bridge::build(&missing).is_err());
        missing = work.clone();
        missing.influence.clear();
        assert!(bridge::build(&missing).is_err());
        missing = work.clone();
        missing.request.context_profile = None;
        assert!(bridge::build(&missing).is_err());
        missing = work.clone();
        missing
            .checked
            .dependencies
            .get_mut("method")
            .unwrap()
            .digest = "forged".into();
        assert!(bridge::build(&missing).is_err());
        let oversized = bridge_work(&format!(
            "String prepare(Task task) {{ /*{}*/ return task; }}",
            "x".repeat(70_000)
        ));
        let error = bridge::build(&oversized).err().unwrap().to_string();
        assert!(error.contains("65536-byte"), "{error}");
    }
    #[test]
    fn guarded_definitions_preserve_both_values_and_stable_local_digest() {
        let text = "String prepare(Task task) { String chosen = task; if (chosen == null) { chosen = \"anonymous\"; } String transformed = chosen; return transformed; }";
        let mut e = evidence(text);
        let parsed = Parsed::new(text).unwrap();
        local_facts(&mut e, &parsed);
        let state = project(&e, &node()).unwrap();
        let transformed: Vec<_> = state
            .definitions
            .iter()
            .filter(|d| {
                d.storage
                    .as_ref()
                    .is_some_and(|s| s.identity == "local:transformed")
            })
            .collect();
        assert_eq!(transformed.len(), 1);
        match &transformed[0].value {
            DataValue::Read { alternatives, .. } => {
                assert_eq!(alternatives.len(), 1);
                let join = state
                    .definitions
                    .iter()
                    .find(|d| d.id == alternatives[0])
                    .unwrap();
                let DataValue::Choice { alternatives } = &join.value else {
                    panic!("guarded choice expected");
                };
                assert_eq!(alternatives.len(), 2);
                assert_ne!(alternatives[0].conditions, alternatives[1].conditions);
                let defs: Vec<_> = alternatives
                    .iter()
                    .flat_map(|a| a.definitions.iter())
                    .map(|id| state.definitions.iter().find(|d| &d.id == id).unwrap())
                    .collect();
                assert!(defs.iter().any(
                    |d| matches!(&d.value,DataValue::Literal {text} if text=="\"anonymous\"")
                ));
            }
            _ => panic!("copy expected"),
        }
        assert_eq!(
            state
                .definitions
                .iter()
                .filter(|d| d.storage.is_none())
                .count(),
            1
        );
        let first = semantic_digest(&state).unwrap();
        let mut changed = state.clone();
        for d in &mut changed.definitions {
            d.citation_id = Some("different-frozen-source".into());
        }
        assert_eq!(first, semantic_digest(&changed).unwrap());
        changed.definitions[0].value = DataValue::Literal {
            text: "different".into(),
        };
        assert_ne!(first, semantic_digest(&changed).unwrap());
    }
    #[test]
    fn sequential_branches_join_shared_refs_without_path_product() {
        fn fixture(count: usize) -> NodeDataState {
            let text = format!(
                "int prepare(int condition) {{ int value = 0; {} return value; }}",
                (1..=count)
                    .map(|i| format!("if (condition == {i}) {{ value = {i}; }}"))
                    .collect::<String>()
            );
            let mut e = evidence(&text);
            let parsed = Parsed::new(&text).unwrap();
            local_facts(&mut e, &parsed);
            // Explicit synthetic compiler access identities for this test input.
            for n in nodes(parsed.callable(&e.observations["method"]).unwrap())
                .into_iter()
                .filter(|n| n.kind() == "identifier" && parsed.text(*n) == "condition")
            {
                let (start, end) = parsed.range(n);
                fact(
                    &mut e,
                    start,
                    end,
                    "parameter:condition",
                    "PARAMETER",
                    false,
                );
            }
            project(&e, &node()).unwrap()
        }
        let small = fixture(30);
        let large = fixture(60);
        assert!(large.definitions.len() > small.definitions.len());
        assert!(
            large.definitions.len() <= 2 * small.definitions.len(),
            "doubling branches must grow the shared DAG linearly"
        );
        for (count, state) in [(30, small), (60, large)] {
            let by_id: BTreeMap<_, _> = state
                .definitions
                .iter()
                .map(|d| (d.id.clone(), d))
                .collect();
            assert_eq!(
                by_id.len(),
                state.definitions.len(),
                "definition IDs must be unique"
            );
            let choices: Vec<_> = state
                .definitions
                .iter()
                .filter(|d| matches!(d.value, DataValue::Choice { .. }))
                .collect();
            assert_eq!(
                choices.len(),
                count,
                "each source branch gets one shared value choice"
            );
            for choice in &choices {
                let DataValue::Choice { alternatives } = &choice.value else {
                    unreachable!()
                };
                assert_eq!(alternatives.len(), 2);
                let guards: Vec<_> = alternatives
                    .iter()
                    .map(|a| {
                        assert_eq!(a.conditions.len(), 1);
                        &a.conditions[0]
                    })
                    .collect();
                assert_eq!(guards[0].expression, guards[1].expression);
                assert_ne!(
                    guards[0].holds, guards[1].holds,
                    "both true and false source guards survive"
                );
                for alternative in alternatives {
                    assert_eq!(alternative.definitions.len(), 1);
                    assert!(
                        by_id.contains_key(&alternative.definitions[0]),
                        "choice edge must resolve to an existing definition"
                    );
                }
            }
            let returned: Vec<_> = state
                .definitions
                .iter()
                .filter(|d| d.storage.is_none())
                .collect();
            assert_eq!(returned.len(), 1);
            let DataValue::Read { alternatives, .. } = &returned[0].value else {
                panic!("shared read expected")
            };
            assert_eq!(
                alternatives,
                &vec![choices.last().unwrap().id.clone()],
                "final return references latest choice, without flattening historical paths"
            );
            let mut pending = alternatives.clone();
            let mut visited = BTreeSet::new();
            let mut terminals = BTreeSet::new();
            while let Some(id) = pending.pop() {
                if !visited.insert(id.clone()) {
                    continue;
                }
                let definition = by_id.get(&id).expect("every reachable edge resolves");
                match &definition.value {
                    DataValue::Choice { alternatives } => pending.extend(
                        alternatives
                            .iter()
                            .flat_map(|a| a.definitions.iter().cloned()),
                    ),
                    DataValue::Literal { .. } => {
                        terminals.insert(id);
                    }
                    _ => panic!("value DAG must contain only guarded choices and source writes"),
                }
            }
            let expected: BTreeSet<_> = state
                .definitions
                .iter()
                .filter(|d| {
                    d.storage
                        .as_ref()
                        .is_some_and(|s| s.identity == "local:value")
                        && matches!(d.value, DataValue::Literal { .. })
                })
                .map(|d| d.id.clone())
                .collect();
            assert_eq!(
                expected.len(),
                count + 1,
                "initializer plus every source write retained"
            );
            assert_eq!(
                terminals, expected,
                "unique-node DAG walk reaches initializer and every write"
            );
            let literals: BTreeSet<_> = terminals
                .iter()
                .map(|id| match &by_id[id].value {
                    DataValue::Literal { text } => text.clone(),
                    _ => unreachable!(),
                })
                .collect();
            assert_eq!(
                literals,
                (0..=count).map(|i| i.to_string()).collect::<BTreeSet<_>>()
            );
        }
    }
    #[test]
    fn exact_column_joins_distinguish_same_line_crlf_unicode_and_reject_tampering() {
        let raw = "String prepare() {\r\n String value = \"λ☕\"; return value + value;\r\n}";
        let normalized = raw.replace("\r\n", "\n");
        let mut e = evidence(raw);
        let first = raw.find("return value").unwrap() + 7;
        let second = raw.find("+ value").unwrap() + 2;
        fact(
            &mut e,
            first,
            first + 5,
            "local:first",
            "LOCAL_VARIABLE",
            false,
        );
        fact(
            &mut e,
            second,
            second + 5,
            "local:second",
            "LOCAL_VARIABLE",
            false,
        );
        let body = source(&normalized);
        let rows: Vec<_> = e
            .observations
            .values()
            .filter(|o| o.kind == "VARIABLE_ACCESS")
            .collect();
        let a = site_range(rows[0], &e, &body).unwrap();
        let b = site_range(rows[1], &e, &body).unwrap();
        assert_ne!(a, b);
        assert_eq!(&normalized[a.0..a.1], "value");
        assert_eq!(&normalized[b.0..b.1], "value");
        let mut forged = rows[0].clone();
        forged.normalized["variableSite"]["sourceByteStart"] = json!(1);
        forged.digest = digest(&forged.normalized).unwrap();
        assert!(site_range(&forged, &e, &body).is_none());
        forged = rows[0].clone();
        forged.normalized["variableSite"]
            .as_object_mut()
            .unwrap()
            .remove("sourceByteStart");
        forged.digest = digest(&forged.normalized).unwrap();
        assert!(site_range(&forged, &e, &body).is_none());
    }
    #[test]
    fn receiver_storage_does_not_alias_field_declaration_or_shadowed_local() {
        let text = "String prepare(Task task, Task other) { this.name = \"own\"; task.name = \"task\"; return other.name; }";
        let mut e = evidence(text);
        let parsed = Parsed::new(text).unwrap();
        local_facts(&mut e, &parsed);
        let normalized = json!({"modifiers":[],"declarationKind":"FIELD","scope":"scope","symbolIdentity":"field:name"});
        e.observations.insert(
            "declaration-field:name".into(),
            Observation {
                id: "declaration-field:name".into(),
                kind: "SYMBOL".into(),
                service: "fixture".into(),
                symbol: "field:name".into(),
                digest: digest(&normalized).unwrap(),
                normalized,
                source_ids: vec![],
            },
        );
        for n in nodes(parsed.callable(&e.observations["method"]).unwrap())
            .into_iter()
            .filter(|n| n.kind() == "field_access")
        {
            let (s, t) = parsed.range(n);
            fact(&mut e, s, t, "field:name", "FIELD", false);
        }
        let state = project(&e, &node()).unwrap();
        let own = state
            .definitions
            .iter()
            .find(|d| {
                d.storage
                    .as_ref()
                    .is_some_and(|s| s.receiver.as_deref() == Some("THIS"))
            })
            .unwrap();
        let other = state
            .definitions
            .iter()
            .find(|d| d.storage.is_none())
            .unwrap();
        match &other.value {
            DataValue::Read {
                storage,
                alternatives,
            } => {
                assert_eq!(storage.receiver.as_deref(), Some("INPUT:parameter:other"));
                assert!(alternatives.is_empty());
                assert_ne!(Some(storage), own.storage.as_ref());
            }
            _ => panic!("field read expected"),
        };
        assert_eq!(state.field_declarations, vec!["declaration-field:name"]);
        assert!(
            state
                .gaps
                .iter()
                .any(|g| g.code == "RECEIVER_STORAGE_OPAQUE")
        );
        let a = DataStorage {
            identity: "local:body/1".into(),
            kind: "LOCAL_VARIABLE".into(),
            receiver: None,
        };
        let b = DataStorage {
            identity: "local:body/2/1".into(),
            ..a.clone()
        };
        let mut env = Env::new();
        env.insert(a.clone(), vec!["outer".into()]);
        env.insert(b, vec!["inner".into()]);
        assert_eq!(env[&a], vec!["outer"]);
    }
    #[test]
    fn conditional_field_write_keeps_opaque_incoming_false_alternative() {
        let text = "String prepare(Task task) { if (task != null) { this.name = \"x\"; } return this.name; }";
        let mut e = evidence(text);
        let parsed = Parsed::new(text).unwrap();
        local_facts(&mut e, &parsed);
        field_facts(&mut e, &parsed);
        let state = project(&e, &node()).unwrap();
        let returned = state
            .definitions
            .iter()
            .find(|d| d.storage.is_none())
            .unwrap();
        let DataValue::Read { alternatives, .. } = &returned.value else {
            panic!("field read expected");
        };
        let choice = state
            .definitions
            .iter()
            .find(|d| d.id == alternatives[0])
            .unwrap();
        let DataValue::Choice { alternatives } = &choice.value else {
            panic!("field choice expected");
        };
        assert_eq!(alternatives.len(), 2);
        assert!(alternatives.iter().any(|a| a.definitions.iter().any(|id| {
            state
                .definitions
                .iter()
                .any(|d| &d.id == id && matches!(d.value, DataValue::Input { .. }))
        })));
    }
    #[test]
    fn opaque_nested_write_stops_local_transfer_and_calls_invalidate_fields() {
        let text = "int prepare() { int value = 0; f(value = 1); return value; }";
        let mut e = evidence(text);
        let parsed = Parsed::new(text).unwrap();
        local_facts(&mut e, &parsed);
        let state = project(&e, &call_node(&e, &parsed)).unwrap();
        assert!(state.definitions.iter().all(|d| d.storage.is_some()));
        assert!(state.gaps.iter().any(|g| g.code == "OPAQUE_EXPRESSION"));
        let text = "String prepare() { this.name = \"before\"; f(); return this.name; }";
        let mut e = evidence(text);
        let parsed = Parsed::new(text).unwrap();
        field_facts(&mut e, &parsed);
        let state = project(&e, &call_node(&e, &parsed)).unwrap();
        let returned = state
            .definitions
            .iter()
            .find(|d| d.storage.is_none())
            .unwrap();
        let DataValue::Read { alternatives, .. } = &returned.value else {
            panic!("field read expected");
        };
        assert!(alternatives.iter().all(|id| {
            state
                .definitions
                .iter()
                .any(|d| &d.id == id && matches!(d.value, DataValue::Interference { .. }))
        }));
        assert!(
            state
                .gaps
                .iter()
                .any(|g| g.code == "CALL_FIELD_INTERFERENCE")
        );
    }
    #[test]
    fn untransferred_initializer_effects_withhold_following_field_state() {
        for (text, missing_join, expected_gap) in [
            (
                "String prepare() { this.name = \"old\"; String a = f(), b = \"x\"; return this.name; }",
                false,
                "OPAQUE_MULTI_DECLARATION",
            ),
            (
                "String prepare() { this.name = \"old\"; String a = f(); return this.name; }",
                true,
                "DATA_SITE_UNAVAILABLE",
            ),
        ] {
            let mut e = evidence(text);
            let parsed = Parsed::new(text).unwrap();
            local_facts(&mut e, &parsed);
            field_facts(&mut e, &parsed);
            if missing_join {
                e.observations.retain(|_, o| {
                    !(o.kind == "VARIABLE_DECLARATION"
                        && o.normalized["variableIdentity"] == "local:a")
                });
            }
            let state = project(&e, &call_node(&e, &parsed)).unwrap();
            assert!(
                state.calls.is_empty(),
                "{text}: skipped initializer must not be admitted"
            );
            assert!(
                state.definitions.iter().all(|d| d.storage.is_some()),
                "{text}: later return must be withheld"
            );
            assert!(
                state.gaps.iter().any(|g| g.code == expected_gap),
                "{text}: {state:?}"
            );
        }
    }
    #[test]
    fn receiver_and_index_effects_withhold_later_transfer() {
        for text in [
            "String prepare() { this.name = \"old\"; get().name = \"x\"; return this.name; }",
            "String prepare() { this.name = \"old\"; return get().name; }",
            "int prepare(int[] arr) { int value = 0; arr[f()] = value; return value; }",
        ] {
            let mut e = evidence(text);
            let parsed = Parsed::new(text).unwrap();
            local_facts(&mut e, &parsed);
            field_facts(&mut e, &parsed);
            let state = project(&e, &call_node(&e, &parsed)).unwrap();
            assert!(
                state.calls.is_empty(),
                "{text}: unsupported receiver/index must not promote its calls"
            );
            assert!(
                state.definitions.iter().all(|d| d.storage.is_some()),
                "{text}: enclosing/later return must be withheld"
            );
            assert!(
                state.gaps.iter().any(|g| matches!(
                    g.code.as_str(),
                    "OPAQUE_RECEIVER_EFFECTS" | "OPAQUE_ASSIGNMENT_TARGET_EFFECTS"
                )),
                "{text}: {state:?}"
            );
        }
    }
    #[test]
    fn terminated_children_do_not_admit_siblings_enclosing_calls_or_returns() {
        for text in [
            "int prepare() { int value = 0; f(value = 1, g()); return value; }",
            "int prepare() { int value = 0; return f(value = 1); }",
            "int prepare() { int value = 0; return (value = 1) + g(); }",
            "int prepare() { int value = 0; int request = f(value = 1); return value; }",
            "int prepare() { int value = 0; return (value = 1).f(g()); }",
        ] {
            let mut e = evidence(text);
            let parsed = Parsed::new(text).unwrap();
            local_facts(&mut e, &parsed);
            let state = project(&e, &call_node(&e, &parsed)).unwrap();
            assert!(
                state.calls.is_empty(),
                "{text}: no sibling or enclosing call after withheld child"
            );
            assert!(
                state.definitions.iter().all(|d| d.storage.is_some()),
                "{text}: no enclosing return"
            );
            assert!(
                !state.definitions.iter().any(|d| d
                    .storage
                    .as_ref()
                    .is_some_and(|s| s.identity == "local:request")),
                "{text}: no enclosing initializer definition"
            );
            assert!(
                state.gaps.iter().any(|g| g.code == "OPAQUE_EXPRESSION"),
                "{text}: {state:?}"
            );
        }
    }
    #[test]
    fn default_selector_and_optional_projection_bytes_remain_omitted() {
        let base =
            json!({"id":"p","service":"s","endpointDeclaration":"e","workerDeclaration":"w"});
        let a: Selection = serde_json::from_value(base.clone()).unwrap();
        let mut explicit = base;
        explicit["expandDataState"] = json!(false);
        let b: Selection = serde_json::from_value(explicit).unwrap();
        assert_eq!(digest(&a).unwrap(), digest(&b).unwrap());
        assert!(
            serde_json::to_value(a)
                .unwrap()
                .get("expandDataState")
                .is_none()
        );
        let mut required = b.clone();
        required.expand_data_state = true;
        let checked = crate::documentation::check::assemble(
            "input".into(),
            BTreeMap::new(),
            BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
        .unwrap();
        assert!(
            super::super::project::project_unresolved(&checked, &[required])
                .unwrap_err()
                .to_string()
                .contains("requires expandSourceCalls")
        );
        assert!(
            serde_json::to_value(node())
                .unwrap()
                .get("dataState")
                .is_none()
        );
    }
}
