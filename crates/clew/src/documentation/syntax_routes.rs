//! Bounded direct Fastify registration syntax; never compiler or activation proof.
use super::*;
use std::collections::BTreeSet;

fn literal(node: Node<'_>, text: &str) -> Option<String> {
    if node.kind() != "string" {
        return None;
    }
    let raw = spelling(node, text);
    let quote = raw.as_bytes().first()?;
    if !matches!(quote, b'\'' | b'"') || raw.as_bytes().last()? != quote || raw.len() < 2 {
        return None;
    }
    let value = &raw[1..raw.len() - 1];
    (!value.contains(['\\', '\n', '\r'])).then(|| value.into())
}

fn path(raw: &str) -> Option<String> {
    if !raw.starts_with('/') {
        return None;
    }
    raw.split('/')
        .map(|segment| {
            if let Some(parameter) = segment.strip_prefix(':') {
                (!parameter.is_empty()
                    && parameter
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_'))
                .then(|| format!("{{{parameter}}}"))
            } else {
                (!segment.contains(['*', '?', '#', '{', '}'])
                    && !segment.chars().any(char::is_whitespace))
                .then(|| segment.into())
            }
        })
        .collect::<Option<Vec<_>>>()
        .map(|segments| segments.join("/"))
}

pub(super) fn attach(
    file: &File<'_>,
    root: Node<'_>,
    evidence: &mut ServiceEvidence,
) -> Result<(), ClewError> {
    if root.has_error() {
        return Ok(());
    }
    let mut nodes = Vec::new();
    let mut pending = vec![root];
    while let Some(node) = pending.pop() {
        if nodes.len() >= MAX_NODES {
            return Err(budget());
        }
        nodes.push(node);
        pending.extend(children(node).into_iter().rev());
    }
    let mut bindings = BTreeMap::<String, usize>::new();
    for node in &nodes {
        let name = match node.kind() {
            "variable_declarator"
            | "function_declaration"
            | "function_expression"
            | "function_signature"
            | "class_declaration" => node.child_by_field_name("name"),
            "required_parameter" | "optional_parameter" => node.child_by_field_name("pattern"),
            "catch_clause" => node.child_by_field_name("parameter"),
            "arrow_function" => node.child_by_field_name("parameter"),
            "import_specifier" => node
                .child_by_field_name("alias")
                .or_else(|| node.child_by_field_name("name")),
            "import_clause" => children(*node)
                .into_iter()
                .find(|n| n.kind() == "identifier"),
            _ => None,
        };
        if let Some(name) = name {
            let mut names = vec![name];
            while let Some(name) = names.pop() {
                if matches!(
                    name.kind(),
                    "identifier" | "shorthand_property_identifier_pattern"
                ) {
                    *bindings
                        .entry(spelling(name, file.text).into())
                        .or_default() += 1;
                } else {
                    names.extend(children(name));
                }
            }
        }
    }
    let factories: BTreeMap<_, _> = children(root)
        .into_iter()
        .filter(|n| n.kind() == "import_statement")
        .filter_map(|node| {
            let source = node.child_by_field_name("source")?;
            if literal(source, file.text).as_deref() != Some("fastify") {
                return None;
            }
            let clause = children(node)
                .into_iter()
                .find(|n| n.kind() == "import_clause")?;
            let imported = children(clause)
                .into_iter()
                .find(|n| n.kind() == "identifier")?;
            let name = spelling(imported, file.text).to_owned();
            (bindings.get(&name) == Some(&1)).then_some((name, node))
        })
        .collect();
    let mut apps = BTreeMap::new();
    for declaration in children(root)
        .into_iter()
        .filter(|n| n.kind() == "lexical_declaration")
    {
        if declaration
            .child_by_field_name("kind")
            .map(|n| spelling(n, file.text))
            != Some("const")
        {
            continue;
        }
        for node in children(declaration)
            .into_iter()
            .filter(|n| n.kind() == "variable_declarator")
        {
            let Some(name) = node
                .child_by_field_name("name")
                .filter(|n| n.kind() == "identifier")
            else {
                continue;
            };
            let Some(value) = node
                .child_by_field_name("value")
                .filter(|n| n.kind() == "call_expression")
            else {
                continue;
            };
            let Some(factory) = value
                .child_by_field_name("function")
                .filter(|n| n.kind() == "identifier")
            else {
                continue;
            };
            if let Some(import) = factories.get(spelling(factory, file.text))
                && bindings.get(spelling(name, file.text)) == Some(&1)
            {
                apps.insert(spelling(name, file.text).to_owned(), (node, *import));
            }
        }
    }
    let mut mutated = BTreeSet::new();
    for node in &nodes {
        if matches!(
            node.kind(),
            "assignment_expression" | "augmented_assignment_expression" | "update_expression"
        ) && let Some(left) = node
            .child_by_field_name("left")
            .or_else(|| node.child_by_field_name("argument"))
        {
            let receiver = if matches!(left.kind(), "member_expression" | "subscript_expression") {
                left.child_by_field_name("object").unwrap_or(left)
            } else {
                left
            };
            if receiver.kind() == "identifier" {
                mutated.insert(spelling(receiver, file.text).to_owned());
            }
        }
    }
    let mut handler_index: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (index, entry) in evidence.entrypoints.iter().enumerate() {
        if entry.trigger["owner"] == "package:"
            && entry.symbol.starts_with(&format!("source:{}/", file.path))
            && let Some(name) = entry.trigger["name"].as_str()
        {
            handler_index.entry(name.into()).or_default().push(index);
        }
    }
    for node in nodes.into_iter().filter(|n| n.kind() == "call_expression") {
        let Some(function) = node
            .child_by_field_name("function")
            .filter(|n| n.kind() == "member_expression")
        else {
            continue;
        };
        let Some(object) = function
            .child_by_field_name("object")
            .filter(|n| n.kind() == "identifier")
        else {
            continue;
        };
        let receiver = spelling(object, file.text);
        let Some((factory, import)) = apps.get(receiver) else {
            continue;
        };
        if mutated.contains(receiver) {
            continue;
        }
        if factory
            .child_by_field_name("value")
            .and_then(|n| n.child_by_field_name("function"))
            .is_some_and(|n| mutated.contains(spelling(n, file.text)))
        {
            continue;
        }
        let Some(method) = function.child_by_field_name("property") else {
            continue;
        };
        let method = spelling(method, file.text);
        if !matches!(
            method,
            "get" | "post" | "put" | "patch" | "delete" | "head" | "options"
        ) {
            continue;
        }
        let Some(arguments) = node.child_by_field_name("arguments") else {
            continue;
        };
        let arguments = children(arguments);
        let [route, handler] = arguments.as_slice() else {
            continue;
        };
        if handler.kind() != "identifier"
            || mutated.contains(spelling(*handler, file.text))
            || bindings.get(spelling(*handler, file.text)) != Some(&1)
        {
            continue;
        }
        let Some(raw_path) = literal(*route, file.text) else {
            continue;
        };
        let Some(route_path) = path(&raw_path) else {
            continue;
        };
        let Some(handlers) = handler_index.get(spelling(*handler, file.text)) else {
            continue;
        };
        let [index] = handlers.as_slice() else {
            continue;
        };
        let identity = format!("syntax:{}/fastify-route/{}", file.path, node.start_byte());
        let mut source_ids = Vec::new();
        for (role, occurrence) in [
            ("registration", node),
            ("factory", *factory),
            ("import", *import),
        ] {
            source_ids.push(file.source(
                evidence,
                &format!("{identity}/{role}"),
                occurrence.start_byte(),
                occurrence.end_byte(),
            )?);
        }
        let normalized = json!({"schema":"codeclew-typescript-source-route/1.0","authority":"SOURCE_SYNTAX","framework":"fastify","method":method.to_uppercase(),"path":route_path,"declaredPath":raw_path,"handler":evidence.entrypoints[*index].symbol,"receiver":receiver,"runtimeActivation":"UNVERIFIED"});
        let dependency = observe(
            evidence,
            "SYNTAX_DETAIL",
            &identity,
            normalized.clone(),
            source_ids.clone(),
        )?;
        let entry = &mut evidence.entrypoints[*index];
        entry.kind = "HTTP_ENDPOINT".into();
        entry.trigger["authority"] = json!("SOURCE_REGISTRATION_SYNTAX");
        if !entry.trigger["sourceRoutes"].is_array() {
            entry.trigger["sourceRoutes"] = json!([]);
        }
        entry.trigger["sourceRoutes"]
            .as_array_mut()
            .unwrap()
            .push(normalized);
        let routes = entry.trigger["sourceRoutes"].as_array().unwrap();
        let methods: BTreeSet<_> = routes
            .iter()
            .filter_map(|r| r["method"].as_str().map(str::to_owned))
            .collect();
        let paths: BTreeSet<_> = routes
            .iter()
            .filter_map(|r| r["path"].as_str().map(str::to_owned))
            .collect();
        entry.trigger["methods"] = json!(methods);
        entry.trigger["paths"] = json!(paths);
        entry.source_ids.extend(source_ids);
        entry.dependency_ids.push(dependency);
        entry
            .boundaries
            .retain(|b| b != "TRIGGER_NOT_FRAMEWORK_RESOLVED");
        entry.boundaries.extend([
            "SOURCE_ROUTE_NOT_COMPILER_RESOLVED".into(),
            "RUNTIME_REGISTRATION_UNPROVEN".into(),
        ]);
        entry.source_ids.sort();
        entry.source_ids.dedup();
        entry.boundaries.sort();
        entry.boundaries.dedup();
    }
    Ok(())
}
