//! Pure Spring interpretation over sealed compiler observations.
use clew_facts::{AnnotatedMethod, AnnotationUse, AnnotationValue, JvmAnnotationFacts};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub const MODULE_ID: &str = "spring";
pub const POLICY_VERSION: &str = "spring-annotation-declarations/1.0";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Derivation {
    pub module_id: String,
    pub policy_version: String,
    pub implementation_digest: String,
    pub input_digest: String,
    pub input_authority: String,
    pub input_schema: String,
    pub coverage: String,
}

pub fn implementation_digest() -> String {
    use sha2::{Digest, Sha256};
    format!(
        "sha256:{}",
        hex::encode(Sha256::digest(include_bytes!("lib.rs")))
    )
}

const WEB: &str = "org.springframework.web.bind.annotation.";
const REQUEST: &str = "org.springframework.web.bind.annotation.RequestMapping";
const LISTENER: &str = "org.springframework.kafka.annotation.KafkaListener";
const HANDLER: &str = "org.springframework.kafka.annotation.KafkaHandler";
const SCHEDULED: &str = "org.springframework.scheduling.annotation.Scheduled";
const CONTROLLER: &str = "org.springframework.stereotype.Controller";
const FEIGN: &str = "org.springframework.cloud.openfeign.FeignClient";
const ALIAS: &str = "org.springframework.core.annotation.AliasFor";

#[derive(Clone)]
struct Binding {
    annotation: String,
    chain: Vec<String>,
    attributes: serde_json::Map<String, Value>,
}

/// No compiler callbacks or filesystem access are available to this interpreter.
/// The caller owns cancellation between bounded compilation/declaration inputs.
pub fn analyze(facts: &JvmAnnotationFacts) -> Result<SpringMetadata, String> {
    facts.validate().map_err(str::to_owned)?;
    let input_digest = clew_facts::digest(facts).map_err(|error| error.to_string())?;
    analyze_validated(facts, input_digest)
}

/// Interpret qualified source spelling without changing the compiler contract.
/// The same rule engine handles language-independent annotation declarations.
pub fn analyze_source(
    source: &clew_facts::SourceAnnotationFacts,
) -> Result<SpringMetadata, String> {
    source.validate().map_err(str::to_owned)?;
    let mut boundaries: BTreeSet<_> = source.boundaries.iter().cloned().collect();
    boundaries.insert("SOURCE_NAMES_NOT_COMPILER_RESOLVED".into());
    boundaries.insert("RUNTIME_REGISTRATION_UNPROVEN".into());
    let mut convert = |annotations: &[clew_facts::SourceAnnotationUse]| -> Vec<AnnotationUse> {
        annotations
            .iter()
            .filter_map(|a| {
                let Some(name) = &a.qualified_name else {
                    boundaries.insert("SOURCE_ANNOTATION_NAME_UNRESOLVED".into());
                    return None;
                };
                let mut arguments = a.arguments.clone();
                // A compiler would reject a mismatched enum type; syntax has no such proof.
                if name == REQUEST
                    && arguments
                        .get("method")
                        .is_some_and(|v| !source_request_methods(v))
                {
                    arguments.insert(
                        "method".into(),
                        AnnotationValue::Unresolved {
                            reason: "REQUEST_METHOD_TYPE_UNPROVEN".into(),
                        },
                    );
                    boundaries.insert("REQUEST_METHOD_TYPE_UNPROVEN".into());
                }
                Some(AnnotationUse {
                    type_name: name.clone(),
                    arguments,
                    origin: a.origin.clone(),
                    use_site_target: None,
                })
            })
            .collect()
    };
    let annotations = convert(&source.declaration.annotations);
    let classes = source
        .owners
        .iter()
        .map(|owner| clew_facts::AnnotatedType {
            identity: owner.identity.clone(),
            annotations: convert(&owner.annotations),
            direct_supertypes: vec![],
        })
        .collect();
    let facts = JvmAnnotationFacts {
        schema: source.schema.clone(),
        authority: source.authority.clone(),
        declaration: source.declaration.identity.clone(),
        definitions: BTreeMap::new(),
        types: vec![],
        callables: vec![clew_facts::AnnotatedCallable {
            method: AnnotatedMethod {
                identity: source.declaration.identity.clone(),
                annotations,
                overrides: vec![],
            },
            classes,
            bean_class: None,
            abstract_method: false,
            inherited: false,
            implementation_source: false,
        }],
        boundaries: boundaries.into_iter().collect(),
        coverage: clew_facts::Coverage {
            status: "PARTIAL".into(),
            scope: "SOURCE_DECLARED_ANNOTATIONS_ONLY".into(),
        },
    };
    let mut metadata = analyze_validated(
        &facts,
        clew_facts::digest(source).map_err(|e| e.to_string())?,
    )?;
    metadata.authority = "FRAMEWORK_DERIVED_SOURCE".into();
    Ok(metadata)
}
fn source_request_methods(value: &AnnotationValue) -> bool {
    match value {
        AnnotationValue::Enum { r#type, value } => {
            r#type == "org.springframework.web.bind.annotation.RequestMethod"
                && [
                    "GET", "HEAD", "POST", "PUT", "PATCH", "DELETE", "OPTIONS", "TRACE",
                ]
                .contains(&value.as_str())
        }
        AnnotationValue::Array { values } => values.iter().all(source_request_methods),
        _ => false,
    }
}
fn analyze_validated(
    facts: &JvmAnnotationFacts,
    input_digest: String,
) -> Result<SpringMetadata, String> {
    let mut interpreter = Interpreter {
        facts,
        boundaries: facts.boundaries.iter().cloned().collect(),
        visits: 0,
    };
    let mut entries = Vec::new();
    for callable in &facts.callables {
        let classes = callable
            .classes
            .iter()
            .map(|class| interpreter.expand_all(&class.annotations))
            .collect::<Vec<_>>();
        let bindings = interpreter.method(&callable.method);
        let mappings = family(&bindings, REQUEST);
        let type_mappings = first_family(&classes, REQUEST);
        let controller = classes
            .iter()
            .flatten()
            .any(|binding| binding.annotation == CONTROLLER);
        let outbound = classes
            .first()
            .is_some_and(|bindings| bindings.iter().any(|binding| binding.annotation == FEIGN))
            && !controller;
        if mappings.len() > 1 || type_mappings.len() > 1 {
            interpreter
                .boundaries
                .insert("MULTIPLE_REQUEST_MAPPINGS_ON_ELEMENT".into());
        }
        let before = entries.len();
        let make_entry = |binding: &Binding, kind: &str| SpringEntry {
            kind: kind.into(),
            annotation: binding.annotation.clone(),
            annotation_chain: binding.chain.clone(),
            attributes: binding.attributes.clone(),
            registration: "RUNTIME_CONDITIONAL".into(),
            controller: None,
            class_attributes: None,
            handler_attributes: None,
            target_symbol: callable.inherited.then(|| callable.method.identity.clone()),
            bean_class: callable.bean_class.clone(),
        };
        for mapping in mappings {
            if outbound {
                interpreter
                    .boundaries
                    .insert("OUTBOUND_FEIGN_CLIENT_NOT_SERVER_ENTRYPOINT".into());
                continue;
            }
            if !controller {
                interpreter
                    .boundaries
                    .insert("CONTROLLER_REGISTRATION_UNPROVEN".into());
            }
            let mut entry = make_entry(mapping, "HTTP_ENDPOINT");
            entry.controller = Some(controller);
            entry.class_attributes = Some(
                type_mappings
                    .iter()
                    .map(|binding| binding.attributes.clone())
                    .collect(),
            );
            entries.push(entry);
        }
        for listener in family(&bindings, LISTENER) {
            entries.push(make_entry(listener, "KAFKA_LISTENER"));
        }
        if let Some(handler) = family(&bindings, HANDLER).first() {
            let listeners = first_family(&classes, LISTENER);
            if listeners.is_empty() {
                interpreter
                    .boundaries
                    .insert("KAFKA_HANDLER_WITHOUT_CLASS_LISTENER".into());
            }
            for listener in listeners {
                let mut entry = make_entry(listener, "KAFKA_LISTENER");
                entry.handler_attributes = Some(handler.attributes.clone());
                entries.push(entry);
            }
        }
        for scheduled in family(&bindings, SCHEDULED) {
            entries.push(make_entry(scheduled, "SCHEDULED_JOB"));
        }
        if entries.len() > before {
            if callable.bean_class.is_none() {
                interpreter.boundaries.insert("NO_BEAN_OWNER".into());
            }
            if callable.abstract_method {
                interpreter
                    .boundaries
                    .insert("ABSTRACT_HANDLER_REQUIRES_IMPLEMENTATION".into());
            }
            if callable.inherited && !callable.implementation_source {
                interpreter
                    .boundaries
                    .insert("INHERITED_HANDLER_SOURCE_UNAVAILABLE".into());
            }
        }
        if entries.len() > 2048 {
            entries.truncate(2048);
            interpreter
                .boundaries
                .insert("FRAMEWORK_ENTRY_LIMIT".into());
            break;
        }
    }
    let coverage = if interpreter.boundaries.is_empty() && facts.coverage.status == "COMPLETE" {
        "COMPLETE"
    } else {
        "PARTIAL"
    };
    Ok(SpringMetadata {
        schema: "spring-entrypoints/0.2".into(),
        authority: "FRAMEWORK_DERIVED".into(),
        entries,
        boundaries: interpreter.boundaries.into_iter().collect(),
        derivation: Some(Derivation {
            module_id: MODULE_ID.into(),
            policy_version: POLICY_VERSION.into(),
            implementation_digest: implementation_digest(),
            input_digest,
            input_authority: facts.authority.clone(),
            input_schema: facts.schema.clone(),
            coverage: coverage.into(),
        }),
    })
}

fn family<'a>(bindings: &'a [Binding], name: &str) -> Vec<&'a Binding> {
    bindings
        .iter()
        .filter(|binding| binding.annotation == name)
        .collect()
}
fn first_family<'a>(classes: &'a [Vec<Binding>], name: &str) -> Vec<&'a Binding> {
    classes
        .iter()
        .map(|bindings| family(bindings, name))
        .find(|bindings| !bindings.is_empty())
        .unwrap_or_default()
}

struct Interpreter<'a> {
    facts: &'a JvmAnnotationFacts,
    boundaries: BTreeSet<String>,
    visits: usize,
}

impl Interpreter<'_> {
    fn method(&mut self, method: &AnnotatedMethod) -> Vec<Binding> {
        let mut result = self.expand_all(&method.annotations);
        let direct = result
            .iter()
            .map(|binding| binding.annotation.clone())
            .collect::<BTreeSet<_>>();
        // Every direct overridden declaration is a separate compiler relationship.
        // Select each nearest family, retaining repeatable uses within that family.
        let mut selected = BTreeSet::new();
        for base in &method.overrides {
            let inherited = self.method(base);
            result.extend(
                inherited
                    .iter()
                    .filter(|binding| {
                        !direct.contains(&binding.annotation)
                            && !selected.contains(&binding.annotation)
                    })
                    .cloned(),
            );
            selected.extend(inherited.into_iter().map(|binding| binding.annotation));
        }
        result
    }

    fn expand_all(&mut self, annotations: &[AnnotationUse]) -> Vec<Binding> {
        annotations
            .iter()
            .flat_map(|annotation| self.expand(annotation, &[]))
            .collect()
    }

    fn expand(&mut self, annotation: &AnnotationUse, path: &[String]) -> Vec<Binding> {
        let name = &annotation.type_name;
        if path.contains(name)
            || name.starts_with("kotlin.")
            || name.starts_with("java.lang.annotation.")
        {
            return Vec::new();
        }
        self.visits += 1;
        if self.visits > 32768 || path.len() >= 32 {
            self.boundaries.insert("ANNOTATION_GRAPH_LIMIT".into());
            return Vec::new();
        }
        let mut chain = path.to_vec();
        chain.push(name.clone());
        if let Some(expected) = match name.as_str() {
            "org.springframework.kafka.annotation.KafkaListeners" => Some(LISTENER),
            "org.springframework.scheduling.annotation.Schedules" => Some(SCHEDULED),
            _ => None,
        } {
            let mut nested = Vec::new();
            for value in annotation.arguments.values() {
                nested_annotations(value, &mut nested);
            }
            if nested.is_empty() {
                self.boundaries
                    .insert("UNRESOLVED_REPEATABLE_CONTAINER".into());
            }
            return nested
                .into_iter()
                .flat_map(|child| {
                    if child.type_name != expected {
                        self.boundaries
                            .insert("INVALID_REPEATABLE_CONTAINER".into());
                        Vec::new()
                    } else {
                        self.expand(child, &chain)
                    }
                })
                .collect();
        }
        let attributes = self.arguments(&annotation.arguments);
        let verb = name.strip_prefix(WEB).and_then(|name| match name {
            "GetMapping" => Some("GET"),
            "PostMapping" => Some("POST"),
            "PutMapping" => Some("PUT"),
            "DeleteMapping" => Some("DELETE"),
            "PatchMapping" => Some("PATCH"),
            _ => None,
        });
        if let Some(verb) = verb {
            let mut attributes = self.aliases(attributes);
            attributes.insert("method".into(), json!([verb]));
            return vec![Binding {
                annotation: REQUEST.into(),
                chain,
                attributes,
            }];
        }
        if [REQUEST, LISTENER, HANDLER, SCHEDULED, CONTROLLER, FEIGN].contains(&name.as_str()) {
            return vec![Binding {
                annotation: name.clone(),
                chain,
                attributes: if name == REQUEST {
                    self.aliases(attributes)
                } else {
                    attributes
                },
            }];
        }
        if self.facts.authority == "SOURCE_ANNOTATIONS"
            && name == "org.springframework.web.bind.annotation.RestController"
        {
            // This is a versioned framework declaration rule, not a synthesized
            // compiler annotation definition or proof of bean registration.
            return vec![Binding {
                annotation: CONTROLLER.into(),
                chain,
                attributes,
            }];
        }
        let Some(definition) = self.facts.definitions.get(name).cloned() else {
            self.boundaries
                .insert("ANNOTATION_DECLARATION_UNAVAILABLE".into());
            return Vec::new();
        };
        let metas = definition
            .annotations
            .iter()
            .flat_map(|meta| self.expand(meta, &chain))
            .collect::<Vec<_>>();
        if metas.is_empty() {
            return Vec::new();
        }
        let defaults = definition
            .members
            .iter()
            .filter_map(|(name, member)| {
                member
                    .default_value
                    .as_ref()
                    .map(|value| (name.clone(), value.clone()))
            })
            .collect::<BTreeMap<_, _>>();
        let mut effective = self.arguments(&defaults);
        effective.extend(attributes.clone());
        metas
            .into_iter()
            .map(|mut meta| {
                for (member_name, member) in &definition.members {
                    for alias in member
                        .annotations
                        .iter()
                        .filter(|alias| alias.type_name == ALIAS)
                    {
                        let args = self.arguments(&alias.arguments);
                        let target = args
                            .get("annotation")
                            .and_then(Value::as_str)
                            .unwrap_or(name);
                        let target_name = args
                            .get("attribute")
                            .and_then(Value::as_str)
                            .filter(|name| !name.is_empty())
                            .or_else(|| {
                                args.get("value")
                                    .and_then(Value::as_str)
                                    .filter(|name| !name.is_empty())
                            })
                            .unwrap_or(member_name);
                        if [
                            name.as_str(),
                            "kotlin.Annotation",
                            "java.lang.annotation.Annotation",
                        ]
                        .contains(&target)
                        {
                            if let Some(value) = attributes
                                .get(member_name)
                                .or_else(|| attributes.get(target_name))
                                .or_else(|| effective.get(member_name))
                                .or_else(|| effective.get(target_name))
                                && meta.attributes.contains_key(target_name)
                            {
                                meta.attributes.insert(target_name.into(), value.clone());
                            }
                            if attributes.get(member_name).is_some()
                                && attributes.get(target_name).is_some()
                                && attributes.get(member_name) != attributes.get(target_name)
                            {
                                self.boundaries
                                    .insert("CONFLICTING_ANNOTATION_ALIASES".into());
                            }
                        } else if target == meta.annotation
                            || meta.chain.iter().any(|name| name == target)
                        {
                            if let Some(final_name) = self.alias_destination(
                                target,
                                target_name,
                                &meta.annotation,
                                &mut BTreeSet::new(),
                            ) {
                                if let Some(value) = effective.get(member_name) {
                                    meta.attributes.insert(final_name, value.clone());
                                }
                            } else {
                                self.boundaries.insert("UNRESOLVED_COMPOSED_ALIAS".into());
                            }
                        }
                    }
                }
                // Retain the existing explicitly versioned convention policy.
                for (name, value) in &effective {
                    if name != "value" && meta.attributes.contains_key(name) {
                        meta.attributes.insert(name.clone(), value.clone());
                    }
                }
                if meta.annotation == REQUEST {
                    meta.attributes = self.aliases(meta.attributes);
                }
                meta
            })
            .collect()
    }

    fn alias_destination(
        &mut self,
        annotation: &str,
        attribute: &str,
        root: &str,
        seen: &mut BTreeSet<String>,
    ) -> Option<String> {
        if annotation == root
            || root == REQUEST
                && annotation.strip_prefix(WEB).is_some_and(|name| {
                    [
                        "GetMapping",
                        "PostMapping",
                        "PutMapping",
                        "DeleteMapping",
                        "PatchMapping",
                    ]
                    .contains(&name)
                })
        {
            return Some(
                if root == REQUEST && attribute == "value" {
                    "path"
                } else {
                    attribute
                }
                .into(),
            );
        }
        if seen.len() >= 32 || !seen.insert(format!("{annotation}#{attribute}")) {
            return None;
        }
        let member = self
            .facts
            .definitions
            .get(annotation)?
            .members
            .get(attribute)?
            .clone();
        if let Some(alias) = member
            .annotations
            .iter()
            .find(|alias| alias.type_name == ALIAS)
        {
            let args = self.arguments(&alias.arguments);
            let target = args
                .get("annotation")
                .and_then(Value::as_str)
                .filter(|value| {
                    !["kotlin.Annotation", "java.lang.annotation.Annotation"].contains(value)
                })
                .unwrap_or(annotation);
            let name = args
                .get("attribute")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .or_else(|| {
                    args.get("value")
                        .and_then(Value::as_str)
                        .filter(|value| !value.is_empty())
                })
                .unwrap_or(attribute);
            self.alias_destination(target, name, root, seen)
        } else if self
            .facts
            .definitions
            .get(root)
            .is_some_and(|definition| definition.members.contains_key(attribute))
        {
            Some(attribute.into())
        } else {
            None
        }
    }

    fn arguments(
        &mut self,
        arguments: &BTreeMap<String, AnnotationValue>,
    ) -> serde_json::Map<String, Value> {
        arguments
            .iter()
            .map(|(name, value)| (name.clone(), self.value(value)))
            .collect()
    }
    fn value(&mut self, value: &AnnotationValue) -> Value {
        match value {
            AnnotationValue::Constant { value } => {
                if value
                    .as_str()
                    .is_some_and(|value| value.contains("${") || value.contains("#{"))
                {
                    self.boundaries.insert("RUNTIME_EXPRESSION".into());
                }
                value.clone()
            }
            AnnotationValue::Enum { value, .. } | AnnotationValue::Class { value } => json!(value),
            AnnotationValue::Array { values } => {
                Value::Array(values.iter().map(|value| self.value(value)).collect())
            }
            AnnotationValue::Annotation { value } => {
                json!({"annotation":value.type_name,"attributes":self.arguments(&value.arguments)})
            }
            AnnotationValue::Unresolved { .. } => {
                self.boundaries.insert("UNRESOLVED_ANNOTATION_VALUE".into());
                Value::Null
            }
        }
    }
    fn aliases(
        &mut self,
        mut attributes: serde_json::Map<String, Value>,
    ) -> serde_json::Map<String, Value> {
        let value = attributes
            .get("value")
            .filter(|value| !value.as_array().is_some_and(Vec::is_empty));
        let path = attributes
            .get("path")
            .filter(|value| !value.as_array().is_some_and(Vec::is_empty));
        if value.is_some() && path.is_some() && value != path {
            self.boundaries.insert("CONFLICTING_PATH_ALIASES".into());
        }
        if let Some(selected) = path.or(value).cloned() {
            attributes.insert("path".into(), selected);
        }
        attributes.remove("value");
        attributes
    }
}

fn nested_annotations<'a>(value: &'a AnnotationValue, output: &mut Vec<&'a AnnotationUse>) {
    match value {
        AnnotationValue::Annotation { value } => output.push(value),
        AnnotationValue::Array { values } => {
            for value in values {
                nested_annotations(value, output);
            }
        }
        _ => (),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpringMetadata {
    pub schema: String,
    pub authority: String,
    pub entries: Vec<SpringEntry>,
    pub boundaries: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub derivation: Option<Derivation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpringEntry {
    pub kind: String,
    pub annotation: String,
    pub annotation_chain: Vec<String>,
    pub attributes: serde_json::Map<String, Value>,
    pub registration: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub controller: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class_attributes: Option<Vec<serde_json::Map<String, Value>>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handler_attributes: Option<serde_json::Map<String, Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_symbol: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bean_class: Option<String>,
}

fn strings(value: Option<&Value>) -> Option<Vec<String>> {
    match value {
        None => Some(Vec::new()),
        Some(Value::String(value)) => Some(vec![value.clone()]),
        Some(Value::Array(values)) => values
            .iter()
            .map(|value| value.as_str().map(str::to_owned))
            .collect(),
        _ => None,
    }
}

fn combine_path(parent: &str, child: &str) -> Option<String> {
    if [parent, child]
        .iter()
        .any(|value| value.contains("${") || value.contains("#{"))
    {
        return None;
    }
    // Spring removes a terminal single-segment wildcard when combining paths.
    // Other wildcard combinations need PathPattern/AntPathMatcher configuration.
    let parent = if !child.is_empty() {
        parent.strip_suffix("/*").unwrap_or(parent)
    } else {
        parent
    };
    if parent.contains('*') || parent.contains('?') {
        return None;
    }
    Some(match (parent.is_empty(), child.is_empty()) {
        (true, true) => String::new(),
        (true, false) => {
            if child.starts_with('/') {
                child.to_owned()
            } else {
                format!("/{child}")
            }
        }
        (false, true) => {
            if parent.starts_with('/') {
                parent.to_owned()
            } else {
                format!("/{parent}")
            }
        }
        (false, false) => format!(
            "{}/{}",
            if parent.starts_with('/') {
                parent.to_owned()
            } else {
                format!("/{parent}")
            }
            .trim_end_matches('/'),
            child.trim_start_matches('/')
        ),
    })
}

fn contains_runtime_value(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(value) => value.contains("${") || value.contains("#{"),
        Value::Array(values) => values.iter().any(contains_runtime_value),
        Value::Object(values) => values.values().any(contains_runtime_value),
        _ => false,
    }
}

/// Preserve raw attributes as evidence; this projection is only a convenient
/// Spring-rule-derived description, never evidence that a live bean was registered.
pub fn describe_trigger(entry: &SpringEntry) -> Value {
    let attributes = &entry.attributes;
    match entry.kind.as_str() {
        "HTTP_ENDPOINT" => {
            let empty = serde_json::Map::new();
            let class = entry
                .class_attributes
                .as_ref()
                .and_then(|list| list.first())
                .unwrap_or(&empty);
            let methods = strings(attributes.get("method")).and_then(|method| {
                let class = strings(class.get("method"))?;
                let result: BTreeSet<_> = class.into_iter().chain(method).collect();
                Some(if result.is_empty() {
                    vec!["ANY".to_owned()]
                } else {
                    result.into_iter().collect()
                })
            });
            let paths =
                strings(class.get("path").or(class.get("value"))).and_then(|mut parents| {
                    let mut children = strings(attributes.get("path").or(attributes.get("value")))?;
                    if parents.is_empty() {
                        parents.push(String::new());
                    }
                    if children.is_empty() {
                        children.push(String::new());
                    }
                    if parents.len().checked_mul(children.len())? > 4096 {
                        return None;
                    }
                    let combined = parents
                        .iter()
                        .flat_map(|parent| {
                            children
                                .iter()
                                .map(move |child| combine_path(parent, child))
                        })
                        .collect::<Option<BTreeSet<_>>>()?;
                    Some(combined.into_iter().collect::<Vec<_>>())
                });
            let mut conditions = serde_json::Map::new();
            for key in ["params", "headers"] {
                conditions.insert(
                    key.into(),
                    json!(strings(class.get(key)).and_then(|class| {
                        Some(
                            class
                                .into_iter()
                                .chain(strings(attributes.get(key))?)
                                .collect::<Vec<_>>(),
                        )
                    })),
                );
            }
            for key in ["consumes", "produces"] {
                let method = strings(attributes.get(key));
                conditions.insert(
                    key.into(),
                    json!(match method {
                        Some(ref value) if value.is_empty() => strings(class.get(key)),
                        value => value,
                    }),
                );
            }
            json!({"pathResolution":if paths.is_some(){"DERIVED"}else{"REQUIRES_RUNTIME_OR_PATH_PATTERN_RESOLUTION"},
                "paths":paths,"methods":methods,"conditions":conditions,"authority":"SPRING_ANNOTATION_RULES"})
        }
        "SCHEDULED_JOB" => json!({
            "disabled":if attributes.values().any(contains_runtime_value) {None} else {Some(attributes.get("cron").and_then(Value::as_str) == Some("-"))},
            "timeUnit":attributes.get("timeUnit").cloned().unwrap_or(json!("MILLISECONDS")),
            "configuration":attributes,
            "authority":"SPRING_ANNOTATION_RULES"
        }),
        _ => json!({"configuration":attributes,"authority":"SPRING_ANNOTATION_RULES"}),
    }
}

#[cfg(test)]
mod source_rules_tests {
    use super::*;
    use clew_facts::{Origin, SourceAnnotatedElement, SourceAnnotationFacts, SourceAnnotationUse};
    fn facts(name: &str, arguments: BTreeMap<String, AnnotationValue>) -> SourceAnnotationFacts {
        SourceAnnotationFacts {
            schema: clew_facts::SOURCE_ANNOTATION_SCHEMA.into(),
            authority: "SOURCE_ANNOTATIONS".into(),
            language: "java".into(),
            declaration: SourceAnnotatedElement {
                identity: "source:Orders/reserve".into(),
                annotations: vec![SourceAnnotationUse {
                    spelling: name.into(),
                    qualified_name: Some(name.into()),
                    qualification: "FULLY_QUALIFIED".into(),
                    arguments,
                    origin: Origin {
                        kind: "SOURCE".into(),
                        identity: "Orders.java".into(),
                        start: None,
                        end: None,
                    },
                }],
            },
            owners: vec![],
            imports: vec![],
            boundaries: vec![],
        }
    }
    #[test]
    fn source_defaults_and_unavailable_composition_remain_distinct() {
        let source = facts(
            "org.springframework.web.bind.annotation.PostMapping",
            BTreeMap::new(),
        );
        let metadata = analyze_source(&source).unwrap();
        assert_eq!(metadata.entries.len(), 1);
        assert_eq!(
            describe_trigger(&metadata.entries[0])["methods"],
            json!(["POST"])
        );
        assert_eq!(describe_trigger(&metadata.entries[0])["paths"], json!([""]));
        assert_eq!(metadata.authority, "FRAMEWORK_DERIVED_SOURCE");
        assert_eq!(
            metadata.derivation.unwrap().input_authority,
            "SOURCE_ANNOTATIONS"
        );
        let custom = analyze_source(&facts("example.FastEndpoint", BTreeMap::new())).unwrap();
        assert!(custom.entries.is_empty());
        assert!(
            custom
                .boundaries
                .contains(&"ANNOTATION_DECLARATION_UNAVAILABLE".into())
        );
    }
    #[test]
    fn source_enum_spelling_is_not_a_resolved_request_method_type() {
        let source = facts(
            REQUEST,
            BTreeMap::from([(
                "method".into(),
                AnnotationValue::Enum {
                    r#type: "example.Other".into(),
                    value: "GET".into(),
                },
            )]),
        );
        let metadata = analyze_source(&source).unwrap();
        assert!(
            metadata
                .boundaries
                .contains(&"REQUEST_METHOD_TYPE_UNPROVEN".into())
        );
        assert!(describe_trigger(&metadata.entries[0])["methods"].is_null());
        let mut forged = source;
        forged.authority = "JAVAC_RESOLVED_ANNOTATIONS".into();
        assert!(analyze_source(&forged).is_err());
    }
}
