//! The Java adapter owns grammar names and the javac identity representation.
//! No Java parser nodes or parameter identity parsing enter the transfer engine.
use super::*;
use crate::{
    canonical::hash_bytes,
    documentation::digest,
    documentation::static_pages::{model::SourceCallNode, source::Parsed},
    documentation::{
        invalid,
        model::{Observation, ServiceEvidence},
    },
};

fn children(node: tree_sitter::Node<'_>) -> Vec<tree_sitter::Node<'_>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).collect()
}

fn kind(node: tree_sitter::Node<'_>) -> Kind {
    match node.kind() {
        "identifier" => Kind::Variable,
        "field_access" => Kind::Field,
        "this" => Kind::This,
        "super" => Kind::Super,
        "string_literal"
        | "character_literal"
        | "decimal_integer_literal"
        | "hex_integer_literal"
        | "decimal_floating_point_literal"
        | "true"
        | "false"
        | "null_literal" => Kind::Literal,
        "parenthesized_expression" => Kind::Parenthesized,
        "unary_expression" => Kind::Unary,
        "binary_expression" => Kind::Binary,
        "method_invocation" => Kind::Call,
        "object_creation_expression" => Kind::Construct,
        "block" => Kind::Block,
        "if_statement" => Kind::If,
        "local_variable_declaration" => Kind::Local,
        "variable_declarator" => Kind::Declarator,
        "assignment_expression" => Kind::Assignment,
        "expression_statement" => Kind::Expression,
        "return_statement" => Kind::Return,
        "throw_statement" => Kind::Throw,
        "lambda_expression" | "class_body" | "class_declaration" => Kind::NestedBody,
        _ => Kind::Unsupported,
    }
}

fn syntax(
    input: &mut Input<'_>,
    parsed: &Parsed,
    node: tree_sitter::Node<'_>,
    depth: usize,
) -> Result<usize, ClewError> {
    if input.nodes.len() >= super::super::MAX_ROWS || depth > 128 {
        return Err(invalid(
            "expandDataState exceeds bounded syntax budget; narrow the selection",
        ));
    }
    let range = parsed.range(node);
    if input.source.text.get(range.0..range.1).is_none() {
        return Err(invalid(
            "data-state syntax is outside retained callable bytes",
        ));
    }
    let index = input.nodes.len();
    input.nodes.push(SyntaxNode {
        kind: kind(node),
        range,
        children: vec![],
        roles: BTreeMap::new(),
        actuals: vec![],
        default_arguments: vec![],
    });
    // Each AST node is stored once. Role links and actuals refer to the same
    // arena entries, avoiding recursive copies of argument/operand subtrees.
    let mut by_id = BTreeMap::new();
    for child in children(node) {
        let child_index = syntax(input, parsed, child, depth + 1)?;
        by_id.insert(child.id(), child_index);
        input.nodes[index].children.push(child_index);
    }
    for (name, role) in [
        ("object", Role::Receiver),
        ("operand", Role::Operand),
        ("operator", Role::Operator),
        ("left", Role::Left),
        ("right", Role::Right),
        ("condition", Role::Condition),
        ("consequence", Role::Then),
        ("alternative", Role::Else),
        ("value", Role::Initializer),
    ] {
        if let Some(child) = node.child_by_field_name(name) {
            let child_index = match by_id.get(&child.id()) {
                Some(&existing) => existing,
                None => syntax(input, parsed, child, depth + 1)?,
            };
            input.nodes[index].roles.insert(role, child_index);
        }
    }
    if matches!(kind(node), Kind::Call | Kind::Construct)
        && let Some(arguments) = node.child_by_field_name("arguments")
        && let Some(&arguments_index) = by_id.get(&arguments.id())
    {
        input.nodes[index].actuals = input.nodes[arguments_index]
            .children
            .iter()
            .enumerate()
            .map(|(slot, &expression)| Actual {
                expression,
                formal_slot: Some(slot),
            })
            .collect();
    }
    Ok(index)
}

fn variable(observation: &Observation) -> Option<Variable> {
    let normalized = &observation.normalized;
    let identity = normalized["variableIdentity"].as_str()?.to_owned();
    if identity.is_empty() {
        return None;
    }
    let kind = match normalized["variableKind"].as_str()? {
        "PARAMETER" => VariableKind::Parameter,
        "LOCAL_VARIABLE" => VariableKind::Local,
        "FIELD" => VariableKind::Field,
        _ => return None,
    };
    // In schema 1.0 javac's exact parameter identity carries the formal slot.
    // Decode that producer contract only here, with its complete owner prefix.
    // Other producers must supply their own slot, never mimic this spelling.
    let formal_slot = (kind == VariableKind::Parameter)
        .then(|| {
            let owner = normalized["variableOwnerIdentity"].as_str()?;
            identity
                .strip_prefix(&format!("parameter:{owner}/slot/"))?
                .parse()
                .ok()
        })
        .flatten();
    Some(Variable {
        identity,
        kind,
        declaration: observation.kind == "VARIABLE_DECLARATION",
        declaration_id: normalized["declarationObservationId"]
            .as_str()
            .map(str::to_owned),
        formal_slot,
    })
}

pub(in super::super) fn prepare<'a>(
    evidence: &'a ServiceEvidence,
    node: &SourceCallNode,
) -> Result<Prepared<'a>, ClewError> {
    let declaration = &evidence.observations[&node.callable.declaration_id];
    let source = declaration
        .source_ids
        .iter()
        .filter_map(|id| evidence.sources.get(id))
        .find(|source| {
            Parsed::new(&source.text)
                .and_then(|p| p.callable(declaration).map(|_| ()))
                .is_some()
        });
    let Some(source) = source else {
        return Ok(Prepared::Unavailable);
    };
    let parsed = Parsed::new(&source.text)
        .ok_or_else(|| invalid("data-state callable parser unavailable"))?;
    let callable = parsed
        .callable(declaration)
        .ok_or_else(|| invalid("data-state callable is ambiguous"))?;
    if callable.has_error() {
        return Ok(Prepared::Partial);
    }
    let mut input = Input {
        source,
        nodes: vec![],
        body: None,
        variables: BTreeMap::new(),
        calls: BTreeMap::new(),
        missing_sites: false,
    };
    for edge in &node.calls {
        let Some(call) = &edge.call else {
            continue;
        };
        let Some(citation) = node.citations.get(&call.citation_id) else {
            continue;
        };
        let Some(occurrence) = edge.occurrence_path.as_ref().filter(|s| !s.is_empty()) else {
            continue;
        };
        if citation.source_id != source.id {
            continue;
        }
        let range = (citation.start_byte, citation.end_byte);
        if range.0 >= range.1 || source.text.get(range.0..range.1).is_none() {
            continue;
        }
        let binding = BoundCall {
            occurrence: occurrence.clone(),
            target_node: edge.target_node.clone(),
            status: edge.status.clone(),
        };
        if let Some(previous) = input.calls.insert(range, binding.clone())
            && previous != binding
        {
            return Err(invalid("data-state call occurrence is ambiguous"));
        }
    }
    let mut bytes = 0;
    for observation in evidence.observations.values().filter(|o| {
        matches!(o.kind.as_str(), "VARIABLE_DECLARATION" | "VARIABLE_ACCESS")
            && o.normalized["callableObservationId"] == node.callable.declaration_id
            && o.normalized["scope"] == node.scope
    }) {
        bytes += serde_json::to_vec(observation)
            .map_err(|_| invalid("data-state fact encoding failed"))?
            .len();
        if bytes > super::super::MAX_BYTES || input.variables.len() >= super::super::MAX_ROWS {
            return Err(invalid(
                "expandDataState exceeds bounded fact budget; narrow the selection",
            ));
        }
        match (
            site_range(observation, evidence, source),
            variable(observation),
        ) {
            (Some(range), Some(variable)) => {
                if input.variables.insert(range, variable).is_some() {
                    return Err(invalid("data-state variable site is ambiguous"));
                }
            }
            _ => input.missing_sites = true,
        }
    }
    if let Some(body) = callable.child_by_field_name("body") {
        input.body = Some(syntax(&mut input, &parsed, body, 0)?);
    }
    Ok(Prepared::Body(input))
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
pub(in super::super) fn site_range(
    o: &Observation,
    e: &ServiceEvidence,
    body: &Source,
) -> Option<(usize, usize)> {
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
