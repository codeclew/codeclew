//! Language syntax normalization. Framework names and rules belong to the shared interpreter.
use super::{analysis, invalid};
use crate::error::ClewError;
use clew_facts::{
    AnnotationValue, Origin, SourceAnnotatedElement, SourceAnnotationFacts, SourceAnnotationUse,
};
use std::collections::{BTreeMap, BTreeSet};
use tree_sitter::Node;

/// File payloads stay reusable. Apply selected-scope package shadowing only after
/// all files have been replayed, so adding a same-package type invalidates a
/// wildcard candidate even when its own file was an unchanged cache hit.
pub(super) fn suppress_package_shadows(
    e: &mut super::model::ServiceEvidence,
) -> Result<(), ClewError> {
    let mut types = BTreeMap::<String, BTreeSet<String>>::new();
    let mut packages = BTreeMap::<String, String>::new();
    for observation in e.observations.values().filter(|o| o.kind == "SYMBOL") {
        let value = &observation.normalized;
        let Some(package) = value["ownerIdentity"]
            .as_str()
            .and_then(|s| s.strip_prefix("package:"))
        else {
            continue;
        };
        for id in &observation.source_ids {
            if let Some(source) = e.sources.get(id) {
                packages.insert(source.file.clone(), package.into());
            }
        }
        if value["syntaxKind"].as_str().is_some_and(is_type)
            && let Some(name) = value["name"].as_str()
        {
            types.entry(package.into()).or_default().insert(name.into());
        }
    }
    let mut affected = BTreeMap::new();
    for observation in e
        .observations
        .values_mut()
        .filter(|o| o.kind == "SOURCE_ANNOTATIONS")
    {
        let package = observation
            .source_ids
            .iter()
            .filter_map(|id| e.sources.get(id))
            .find_map(|s| packages.get(&s.file));
        let Some(local) = package.and_then(|p| types.get(p)) else {
            continue;
        };
        let mut facts: SourceAnnotationFacts =
            serde_json::from_value(observation.normalized["facts"].clone())
                .map_err(super::io_error)?;
        let mut changed = false;
        for element in std::iter::once(&mut facts.declaration).chain(&mut facts.owners) {
            for annotation in &mut element.annotations {
                if annotation.qualification == "WILDCARD_IMPORT_CANDIDATE"
                    && local.contains(&annotation.spelling)
                {
                    annotation.qualified_name = None;
                    annotation.qualification = "UNRESOLVED".into();
                    changed = true;
                }
            }
        }
        if changed {
            facts
                .boundaries
                .push("PACKAGE_LOCAL_ANNOTATION_SHADOWS_WILDCARD".into());
            facts.boundaries.sort();
            facts.boundaries.dedup();
            let metadata = clew_framework_spring::analyze_source(&facts).map_err(invalid)?;
            observation.normalized["facts"] =
                serde_json::to_value(facts).map_err(super::io_error)?;
            observation.normalized["framework"] =
                serde_json::to_value(&metadata).map_err(super::io_error)?;
            observation.digest = super::digest(&observation.normalized)?;
            e.boundaries.extend(metadata.boundaries.clone());
            affected.insert(observation.symbol.clone(), metadata);
        }
    }
    for entry in &mut e.entrypoints {
        if let Some(metadata) = affected.get(&entry.symbol) {
            super::syntax::project_source_framework(entry, metadata);
        }
    }
    Ok(())
}

pub(super) struct Context<'a> {
    language: &'a str,
    file: &'a str,
    text: &'a str,
    imports: Vec<String>,
    names: BTreeMap<String, BTreeSet<String>>,
    local_types: BTreeSet<String>,
    local_callables: BTreeSet<String>,
    package: String,
}
fn children(node: Node<'_>) -> Vec<Node<'_>> {
    let mut c = node.walk();
    node.named_children(&mut c).collect()
}
fn spelling<'a>(node: Node<'_>, text: &'a str) -> &'a str {
    &text[node.byte_range()]
}
fn is_type(kind: &str) -> bool {
    matches!(
        kind,
        "class_declaration"
            | "interface_declaration"
            | "annotation_type_declaration"
            | "enum_declaration"
            | "record_declaration"
            | "object_declaration"
    )
}
impl<'a> Context<'a> {
    fn type_name(&self, value: &str, package: &str) -> Option<String> {
        let tokens = analysis::java_tokens(value);
        let mut name = tokens.join("");
        let nullable = name.ends_with('?');
        if nullable {
            name.pop();
        }
        if name.is_empty()
            || !name.split('.').all(|part| {
                part.bytes()
                    .next()
                    .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
                    && part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            })
        {
            return None;
        }
        let qualified = if name.contains('.') {
            name
        } else if self.local_types.contains(&name) {
            qualified_type(package, &name)
        } else if let Some(imports) = self.names.get(&name) {
            if imports.len() != 1 {
                return None;
            }
            imports.first()?.clone()
        } else if [
            "String", "Int", "Long", "Boolean", "Double", "Float", "Byte", "Short", "Char", "Unit",
            "Any",
        ]
        .contains(&name.as_str())
        {
            format!("kotlin.{name}")
        } else if self.imports.iter().any(|import| import.contains('*')) {
            return None;
        } else {
            qualified_type(package, &name)
        };
        Some(format!("{qualified}{}", if nullable { "?" } else { "" }))
    }

    /// Declared Kotlin names and signatures only; this never resolves an override through a compiler.
    pub fn inheritance_metadata(
        &self,
        node: Node<'_>,
        owner: &str,
        name: &str,
    ) -> Option<serde_json::Value> {
        if self.language != "kotlin" {
            return None;
        }
        let end = children(node)
            .into_iter()
            .find(|n| matches!(n.kind(), "class_body" | "function_body"))
            .map_or(node.end_byte(), |n| n.start_byte());
        let header = &self.text[node.start_byte()..end];
        let tokens = analysis::java_tokens(header);
        if node.kind() == "class_declaration" {
            let qualified = qualified_type(
                owner
                    .strip_prefix("class:")
                    .or_else(|| owner.strip_prefix("package:"))
                    .unwrap_or(""),
                name,
            );
            let mut bases = Vec::new();
            for child in children(node).into_iter().filter(|child| {
                matches!(
                    child.kind(),
                    "delegation_specifiers" | "delegation_specifier"
                )
            }) {
                let mut stack = vec![child];
                while let Some(n) = stack.pop() {
                    if n.kind() == "user_type" {
                        bases.push(self.type_name(spelling(n, self.text), &self.package));
                    } else if !matches!(n.kind(), "value_arguments" | "type_arguments") {
                        stack.extend(children(n).into_iter().rev());
                    }
                }
            }
            return Some(
                serde_json::json!({"kind":if tokens.iter().any(|t|t=="interface"){"INTERFACE"}else{"CLASS"},"qualifiedName":qualified,"supertypes":bases,"generic":children(node).iter().any(|n|n.kind()=="type_parameters")}),
            );
        }
        if node.kind() != "function_declaration" || !owner.starts_with("class:") {
            return None;
        }
        let parameters = children(node)
            .into_iter()
            .find(|n| n.kind() == "function_value_parameters")?;
        if children(node).iter().any(|n| {
            n.kind() == "type_parameters"
                || (n.start_byte() < parameters.start_byte()
                    && matches!(n.kind(), "user_type" | "nullable_type"))
        }) {
            return None;
        }
        let mut types = Vec::new();
        for parameter in children(parameters) {
            let tokens = analysis::java_tokens(spelling(parameter, self.text));
            if tokens.iter().any(|t| t == "vararg") {
                return None;
            }
            let mut depth = 0usize;
            let mut colon = None;
            for (i, t) in tokens.iter().enumerate() {
                match t.as_str() {
                    "(" | "[" => depth += 1,
                    ")" | "]" => depth = depth.saturating_sub(1),
                    ":" if depth == 0 => {
                        colon = Some(i);
                        break;
                    }
                    _ => {}
                }
            }
            let after = &tokens[colon? + 1..];
            let end = after.iter().position(|t| t == "=").unwrap_or(after.len());
            types.push(self.type_name(&after[..end].join(""), &self.package)?);
        }
        Some(
            serde_json::json!({"kind":"METHOD","owner":owner.trim_start_matches("class:"),"name":name,"parameterTypes":types,"overrideDeclared":tokens.iter().any(|t|t=="override")}),
        )
    }
    pub fn new(
        language: &'a str,
        file: &'a str,
        text: &'a str,
        root: Node<'_>,
    ) -> Result<Self, ClewError> {
        let mut imports = Vec::new();
        let mut names: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut local_types = BTreeSet::new();
        let mut local_callables = BTreeSet::new();
        for node in children(root)
            .into_iter()
            .filter(|n| matches!(n.kind(), "import_declaration" | "import_header" | "import"))
        {
            let mut tokens = analysis::java_tokens(spelling(node, text));
            tokens.retain(|s| s != ";");
            imports.push(tokens.join(" "));
            if tokens.first().is_none_or(|s| s != "import")
                || tokens.iter().any(|s| s == "static" || s == "*")
            {
                continue;
            }
            let alias = tokens.iter().position(|s| s == "as");
            let stop = alias.unwrap_or(tokens.len());
            let qualified = tokens[1..stop].join("");
            let name = alias
                .and_then(|i| tokens.get(i + 1).cloned())
                .unwrap_or_else(|| qualified.rsplit('.').next().unwrap_or("").into());
            if !name.is_empty() {
                names.entry(name).or_default().insert(qualified);
            }
        }
        let mut stack = vec![root];
        let mut visited = 0;
        while let Some(node) = stack.pop() {
            visited += 1;
            if visited > 200000 {
                return Err(invalid("source annotation syntax budget exceeded"));
            }
            if is_type(node.kind())
                && let Some(name) = node.child_by_field_name("name")
            {
                local_types.insert(spelling(name, text).into());
            }
            if node.kind() == "function_declaration"
                && let Some(name) = node.child_by_field_name("name")
            {
                local_callables.insert(spelling(name, text).into());
            }
            stack.extend(children(node));
        }
        Ok(Self {
            language,
            file,
            text,
            imports,
            names,
            local_types,
            local_callables,
            package: children(root)
                .into_iter()
                .find(|n| matches!(n.kind(), "package_header" | "package_declaration"))
                .map(|n| {
                    analysis::java_tokens(spelling(n, text))
                        .into_iter()
                        .filter(|t| t != "package" && t != ";")
                        .collect::<Vec<_>>()
                        .join("")
                })
                .unwrap_or_default(),
        })
    }
    fn qualify(&self, name: &str) -> Option<(String, String)> {
        if name.contains('.') {
            return Some((name.into(), "FULLY_QUALIFIED".into()));
        }
        if self.local_types.contains(name) {
            return None;
        }
        if let Some(names) = self.names.get(name) {
            return (names.len() == 1)
                .then(|| (names.first().unwrap().clone(), "EXPLICIT_IMPORT".into()));
        }
        clew_framework_spring::source_annotation_candidate(name, &self.imports)
            .map(|name| (name, "WILDCARD_IMPORT_CANDIDATE".into()))
    }
    fn value(&self, tokens: &[String], depth: usize) -> AnnotationValue {
        let unresolved = || AnnotationValue::Unresolved {
            reason: "SOURCE_EXPRESSION_REQUIRES_RESOLUTION".into(),
        };
        if tokens.is_empty() || depth > 16 {
            return unresolved();
        }
        if tokens.len() == 1
            && let Ok(value) = serde_json::from_str::<serde_json::Value>(&tokens[0])
            && !value.is_object()
            && !value.is_array()
            && !value.is_null()
            && !value
                .as_str()
                .is_some_and(|s| self.language == "kotlin" && s.contains('$') && !s.contains("${"))
        {
            return AnnotationValue::Constant { value };
        }
        let (start, end) = if matches!(
            (
                tokens.first().map(String::as_str),
                tokens.last().map(String::as_str)
            ),
            (Some("{"), Some("}")) | (Some("["), Some("]"))
        ) {
            (1, tokens.len() - 1)
        } else if tokens.first().is_some_and(|s| s == "arrayOf")
            && !self.local_types.contains("arrayOf")
            && !self.local_callables.contains("arrayOf")
            && self.names.get("arrayOf").is_none_or(|names| {
                names.len() == 1 && names.first().is_some_and(|name| name == "kotlin.arrayOf")
            })
            && tokens.get(1).is_some_and(|s| s == "(")
            && tokens.last().is_some_and(|s| s == ")")
        {
            (2, tokens.len() - 1)
        } else {
            (0, 0)
        };
        if start > 0 {
            return AnnotationValue::Array {
                values: split(&tokens[start..end], ",")
                    .into_iter()
                    .filter(|t| !t.is_empty())
                    .map(|t| self.value(t, depth + 1))
                    .collect(),
            };
        }
        if tokens.len() >= 3
            && tokens[tokens.len() - 2] == "."
            && tokens.iter().enumerate().all(|(i, t)| {
                if i % 2 == 1 {
                    t == "."
                } else {
                    t.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                }
            })
        {
            let prefix = tokens[..tokens.len() - 2].join("");
            if let Some((r#type, _)) = self.qualify(&prefix) {
                return AnnotationValue::Enum {
                    r#type,
                    value: tokens.last().unwrap().clone(),
                };
            }
        }
        unresolved()
    }
    fn annotation(&self, node: Node<'_>) -> SourceAnnotationUse {
        let tokens = analysis::java_tokens(spelling(node, self.text));
        let mut cursor = usize::from(tokens.first().is_some_and(|t| t == "@"));
        let mut name = String::new();
        while cursor < tokens.len() && tokens[cursor] != "(" && tokens[cursor] != ":" {
            name.push_str(&tokens[cursor]);
            cursor += 1;
        }
        let qualified = if tokens.get(cursor).is_some_and(|t| t == ":") {
            None
        } else {
            self.qualify(&name)
        };
        let mut arguments = BTreeMap::new();
        if tokens.get(cursor).is_some_and(|t| t == "(") && tokens.last().is_some_and(|t| t == ")") {
            for (index, part) in split(&tokens[cursor + 1..tokens.len() - 1], ",")
                .into_iter()
                .enumerate()
            {
                if part.is_empty() {
                    continue;
                }
                let eq = split(part, "=");
                let (key, value) = if eq.len() == 2 && eq[0].len() == 1 {
                    (eq[0][0].clone(), self.value(eq[1], 0))
                } else if index == 0 {
                    ("value".into(), self.value(part, 0))
                } else {
                    (
                        format!("unresolvedPositional{index}"),
                        AnnotationValue::Unresolved {
                            reason: "POSITIONAL_ANNOTATION_ARGUMENT_UNRESOLVED".into(),
                        },
                    )
                };
                if arguments.insert(key.clone(), value).is_some() {
                    arguments.insert(
                        key,
                        AnnotationValue::Unresolved {
                            reason: "DUPLICATE_ANNOTATION_ARGUMENT".into(),
                        },
                    );
                }
            }
        }
        SourceAnnotationUse {
            spelling: if name.is_empty() {
                "unresolved".into()
            } else {
                name
            },
            qualified_name: qualified.as_ref().map(|(n, _)| n.clone()),
            qualification: qualified
                .map(|(_, q)| q)
                .unwrap_or_else(|| "UNRESOLVED".into()),
            arguments,
            origin: Origin {
                kind: "SOURCE".into(),
                identity: self.file.into(),
                start: None,
                end: None,
            },
        }
    }
    fn annotations(&self, node: Node<'_>) -> Vec<SourceAnnotationUse> {
        let mut stack: Vec<_> = children(node)
            .into_iter()
            .filter(|n| {
                n.kind() == "modifiers" || matches!(n.kind(), "annotation" | "marker_annotation")
            })
            .collect();
        let mut result = Vec::new();
        while let Some(node) = stack.pop() {
            if matches!(node.kind(), "annotation" | "marker_annotation") {
                result.push((node.start_byte(), self.annotation(node)));
            } else {
                stack.extend(children(node).into_iter().rev());
            }
        }
        result.sort_by_key(|(offset, _)| *offset);
        result
            .into_iter()
            .map(|(_, annotation)| annotation)
            .collect()
    }
    pub fn facts(
        &self,
        node: Node<'_>,
        identity: &str,
    ) -> Result<SourceAnnotationFacts, ClewError> {
        let mut owners = Vec::new();
        let mut boundaries = Vec::new();
        let mut parent = node.parent();
        while let Some(node) = parent {
            if owners.len() > 128 {
                return Err(invalid("source annotation owner depth exceeded"));
            }
            if is_type(node.kind()) {
                let name = node
                    .child_by_field_name("name")
                    .map(|n| spelling(n, self.text))
                    .unwrap_or("anonymous");
                owners.push(SourceAnnotatedElement {
                    identity: format!("source-owner:{}/{}", self.file, name),
                    annotations: self.annotations(node),
                });
                if children(node).iter().any(|n| {
                    matches!(
                        n.kind(),
                        "superclass"
                            | "super_interfaces"
                            | "delegation_specifier"
                            | "delegation_specifiers"
                            | "explicit_delegation"
                    )
                }) {
                    boundaries.push("SOURCE_INHERITANCE_UNRESOLVED".into());
                }
            }
            parent = node.parent();
        }
        let result = SourceAnnotationFacts {
            schema: clew_facts::SOURCE_ANNOTATION_SCHEMA.into(),
            authority: "SOURCE_ANNOTATIONS".into(),
            language: self.language.into(),
            declaration: SourceAnnotatedElement {
                identity: identity.into(),
                annotations: self.annotations(node),
            },
            owners,
            imports: self.imports.clone(),
            boundaries,
        };
        result.validate().map_err(invalid)?;
        Ok(result)
    }
}
fn split<'a>(tokens: &'a [String], delimiter: &str) -> Vec<&'a [String]> {
    let mut result = Vec::new();
    let mut start = 0;
    let mut depth = 0i32;
    for (i, t) in tokens.iter().enumerate() {
        match t.as_str() {
            "(" | "[" | "{" => depth += 1,
            ")" | "]" | "}" => depth -= 1,
            _ => {}
        }
        if depth == 0 && t == delimiter {
            result.push(&tokens[start..i]);
            start = i + 1;
        }
    }
    result.push(&tokens[start..]);
    result
}

fn qualified_type(owner: &str, name: &str) -> String {
    if owner.is_empty() {
        name.into()
    } else {
        format!("{owner}.{name}")
    }
}
