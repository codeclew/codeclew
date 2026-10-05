//! Pure ASP.NET Core MVC interpretation over sealed CLR attribute observations.
//!
//! The rules follow the default application model: controller and action
//! discovery conventions, attribute-route selectors, route-template combination
//! and token replacement. Programmatic configuration (conventions, custom
//! feature providers, endpoint routing setup, minimal APIs) is not visible in
//! attributes and stays an explicit boundary.
use clew_facts::{ClrAttributeFacts, ClrAttributeUse, ClrValue};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;

pub const MODULE_ID: &str = "aspnetcore";
pub const POLICY_VERSION: &str = "aspnetcore-mvc-attribute-routing/1.0";
pub const OUTPUT_SCHEMA: &str = "aspnetcore-entrypoints/1.0";
const MAX_PATHS: usize = 4096;

const MVC: &str = "class:Microsoft.AspNetCore.Mvc.";
const CONTROLLER: &str = "class:Microsoft.AspNetCore.Mvc.ControllerAttribute";
const NON_CONTROLLER: &str = "class:Microsoft.AspNetCore.Mvc.NonControllerAttribute";
const NON_ACTION: &str = "class:Microsoft.AspNetCore.Mvc.NonActionAttribute";
const ROUTE: &str = "class:Microsoft.AspNetCore.Mvc.RouteAttribute";
const HTTP_METHOD: &str = "class:Microsoft.AspNetCore.Mvc.Routing.HttpMethodAttribute";
const ACCEPT_VERBS: &str = "class:Microsoft.AspNetCore.Mvc.AcceptVerbsAttribute";
const ROUTE_TEMPLATE_PROVIDER: &str =
    "class:Microsoft.AspNetCore.Mvc.Routing.IRouteTemplateProvider";
const HTTP_METHOD_PROVIDER: &str =
    "class:Microsoft.AspNetCore.Mvc.Routing.IActionHttpMethodProvider";
const AREA: &str = "class:Microsoft.AspNetCore.Mvc.AreaAttribute";
const CONTROLLER_NAME: &str = "class:Microsoft.AspNetCore.Mvc.ControllerNameAttribute";
const ACTION_NAME: &str = "class:Microsoft.AspNetCore.Mvc.ActionNameAttribute";
const CONSUMES: &str = "class:Microsoft.AspNetCore.Mvc.ConsumesAttribute";
const PRODUCES: &str = "class:Microsoft.AspNetCore.Mvc.ProducesAttribute";
const AUTHORIZE: &str = "class:Microsoft.AspNetCore.Authorization.AuthorizeAttribute";
const ALLOW_ANONYMOUS: &str = "class:Microsoft.AspNetCore.Authorization.AllowAnonymousAttribute";
const API_VERSION: [&str; 2] = [
    "class:Asp.Versioning.ApiVersionAttribute",
    "class:Microsoft.AspNetCore.Mvc.ApiVersionAttribute",
];
const MAP_TO_API_VERSION: [&str; 2] = [
    "class:Asp.Versioning.MapToApiVersionAttribute",
    "class:Microsoft.AspNetCore.Mvc.MapToApiVersionAttribute",
];
/// Framework verb attributes whose HTTP methods are fixed by their type.
const VERBS: [(&str, &str); 7] = [
    ("HttpGetAttribute", "GET"),
    ("HttpPostAttribute", "POST"),
    ("HttpPutAttribute", "PUT"),
    ("HttpDeleteAttribute", "DELETE"),
    ("HttpPatchAttribute", "PATCH"),
    ("HttpHeadAttribute", "HEAD"),
    ("HttpOptionsAttribute", "OPTIONS"),
];

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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AspNetCoreMetadata {
    pub schema: String,
    pub authority: String,
    pub entries: Vec<AspNetCoreEntry>,
    pub boundaries: Vec<String>,
    pub derivation: Derivation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AspNetCoreEntry {
    pub kind: String,
    pub controller: String,
    pub action: String,
    /// Attribute types that selected this route, in precedence order.
    pub attribute_chain: Vec<String>,
    pub registration: String,
    pub trigger: Value,
}

pub fn implementation_digest() -> String {
    use sha2::{Digest, Sha256};
    format!(
        "sha256:{}",
        hex::encode(Sha256::digest(include_bytes!("lib.rs")))
    )
}

/// The trigger is derived once; this mirrors the Spring package's accessor.
pub fn describe_trigger(entry: &AspNetCoreEntry) -> Value {
    entry.trigger.clone()
}

/// One route selector: an optional template and its HTTP method constraints.
#[derive(Debug, Clone)]
struct Selector {
    template: Option<String>,
    methods: BTreeSet<String>,
    unknown_methods: bool,
    attribute: Option<String>,
}

/// No compiler callbacks or filesystem access are available to this interpreter.
pub fn analyze(facts: &ClrAttributeFacts) -> Result<AspNetCoreMetadata, String> {
    facts.validate().map_err(str::to_owned)?;
    let input_digest = clew_facts::digest(facts).map_err(|error| error.to_string())?;
    let mut boundaries: BTreeSet<String> = facts.boundaries.iter().cloned().collect();
    let derivation = Derivation {
        module_id: MODULE_ID.into(),
        policy_version: POLICY_VERSION.into(),
        implementation_digest: implementation_digest(),
        input_digest,
        input_authority: facts.authority.clone(),
        input_schema: facts.schema.clone(),
        coverage: facts.coverage.status.clone(),
    };
    let mut entries = Vec::new();
    if is_controller(facts) && is_action(facts) {
        entries = action_entries(facts, &mut boundaries);
    }
    Ok(AspNetCoreMetadata {
        schema: OUTPUT_SCHEMA.into(),
        authority: "FRAMEWORK_DERIVED".into(),
        entries,
        boundaries: boundaries.into_iter().collect(),
        derivation,
    })
}

/// Default controller discovery: a non-abstract, non-generic class that is
/// named `*Controller` or carries `[Controller]` (directly, through a derived
/// attribute such as `[ApiController]`, or inherited from `ControllerBase`),
/// and is not marked `[NonController]`.
pub fn is_controller(facts: &ClrAttributeFacts) -> bool {
    let owner = &facts.containing_type;
    !owner.is_abstract
        && !owner.is_generic
        && !owner
            .attributes
            .iter()
            .any(|use_| use_.is_a(NON_CONTROLLER))
        && (owner.name.ends_with("Controller")
            || owner.attributes.iter().any(|use_| use_.is_a(CONTROLLER)))
}

/// Default action discovery: public, instance, non-abstract, non-generic
/// methods without `[NonAction]`.
pub fn is_action(facts: &ClrAttributeFacts) -> bool {
    let method = &facts.method;
    method.accessibility == "PUBLIC"
        && !method.is_static
        && !method.is_abstract
        && !method.is_generic
        && !method.attributes.iter().any(|use_| use_.is_a(NON_ACTION))
        && method.name != "Dispose"
}

fn action_entries(
    facts: &ClrAttributeFacts,
    boundaries: &mut BTreeSet<String>,
) -> Vec<AspNetCoreEntry> {
    let owner = &facts.containing_type;
    if owner.accessibility != "PUBLIC" {
        // The default ControllerFeatureProvider only discovers public types.
        boundaries.insert("CONTROLLER_DISCOVERY_REQUIRES_CUSTOM_FEATURE_PROVIDER".into());
    }
    if owner.is_nested {
        boundaries.insert("NESTED_CONTROLLER_DISCOVERY_REQUIRES_REVIEW".into());
    }
    // Attributes of an overridden action apply when the override declares none.
    let action_attributes: Vec<&ClrAttributeUse> = if facts.method.attributes.is_empty() {
        facts.overridden_attributes.iter().collect()
    } else {
        facts.method.attributes.iter().collect()
    };
    let type_attributes: Vec<&ClrAttributeUse> = owner.attributes.iter().collect();
    let controller_selectors = selectors(&type_attributes, boundaries);
    let action_selectors = selectors(&action_attributes, boundaries);
    let controller_name = string_attribute(&type_attributes, CONTROLLER_NAME, "name")
        .unwrap_or_else(|| {
            owner
                .name
                .strip_suffix("Controller")
                .filter(|name| !name.is_empty())
                .unwrap_or(&owner.name)
                .to_owned()
        });
    // MvcOptions.SuppressAsyncSuffixInActionNames defaults to true; the assumption
    // matters only where a template uses the [action] token.
    let mut async_suffix_assumed = false;
    let action_name = match string_attribute(&action_attributes, ACTION_NAME, "name") {
        Some(name) => name,
        None => match facts.method.name.strip_suffix("Async") {
            Some(trimmed) if !trimmed.is_empty() => {
                async_suffix_assumed = true;
                trimmed.to_owned()
            }
            _ => facts.method.name.clone(),
        },
    };
    let area = string_attribute(&type_attributes, AREA, "areaName");
    let versions = api_versions(&type_attributes, &action_attributes);
    let conditions = conditions(&type_attributes, &action_attributes);
    let attributed = controller_selectors
        .iter()
        .chain(&action_selectors)
        .any(|selector| selector.template.is_some());
    if !attributed {
        boundaries.insert("CONVENTIONAL_ROUTING_REQUIRES_RUNTIME_CONFIGURATION".into());
    }
    let mut entries = Vec::new();
    let controllers: Vec<Option<&Selector>> = if controller_selectors.is_empty() {
        vec![None]
    } else {
        controller_selectors.iter().map(Some).collect()
    };
    for action in &action_selectors {
        let mut paths = BTreeSet::new();
        let mut resolution = "DERIVED";
        let mut chain = Vec::new();
        for controller in &controllers {
            let combined = combine(
                controller.and_then(|selector| selector.template.as_deref()),
                action.template.as_deref(),
            );
            if let Some(selector) = controller.and_then(|selector| selector.attribute.clone()) {
                chain.push(selector);
            }
            let Some(template) = combined else {
                resolution = "REQUIRES_RUNTIME_ROUTE_CONFIGURATION";
                continue;
            };
            if async_suffix_assumed && template.to_ascii_lowercase().contains("[action]") {
                boundaries.insert("ACTION_NAME_ASYNC_SUFFIX_DEFAULT_ASSUMED".into());
            }
            match replace_tokens(&template, &controller_name, &action_name, area.as_deref()) {
                Some(path) => {
                    paths.insert(path);
                }
                None => resolution = "REQUIRES_RUNTIME_ROUTE_RESOLUTION",
            }
        }
        if let Some(attribute) = &action.attribute {
            chain.push(attribute.clone());
        }
        chain.sort();
        chain.dedup();
        let mut version_boundary = false;
        let (paths, versioned) = expand_versions(paths, &versions, &mut version_boundary);
        if version_boundary {
            boundaries.insert("API_VERSION_ROUTE_SEGMENT_FORMAT_RUNTIME_CONFIGURED".into());
        }
        if paths.len() > MAX_PATHS {
            boundaries.insert("ROUTE_PATH_PRODUCT_BUDGET".into());
            resolution = "REQUIRES_RUNTIME_ROUTE_RESOLUTION";
        }
        let methods = if action.unknown_methods {
            boundaries.insert("CUSTOM_HTTP_METHOD_PROVIDER_REQUIRES_REVIEW".into());
            Value::Null
        } else if action.methods.is_empty() {
            json!(["ANY"])
        } else {
            json!(action.methods)
        };
        let path_value = if paths.is_empty() || paths.len() > MAX_PATHS {
            Value::Null
        } else {
            json!(paths)
        };
        let resolution = if path_value.is_null() && resolution == "DERIVED" {
            "REQUIRES_RUNTIME_ROUTE_CONFIGURATION"
        } else {
            resolution
        };
        entries.push(AspNetCoreEntry {
            kind: "HTTP_ENDPOINT".into(),
            controller: owner.identity.clone(),
            action: action_name.clone(),
            attribute_chain: chain,
            registration: "RUNTIME_CONDITIONAL".into(),
            trigger: json!({
                "pathResolution":resolution,
                "paths":path_value,
                "versionedPaths":versioned,
                "methods":methods,
                "conditions":conditions,
                "apiVersions":versions.declared,
                "authority":"ASPNETCORE_ROUTING_RULES",
            }),
        });
    }
    entries
}

/// Attribute-route selectors per the default application model: every templated
/// route provider is one selector; template-less ("silent") verb attributes and
/// `[AcceptVerbs]` constrain every selector; without templates there is one
/// unrouted selector.
fn selectors(attributes: &[&ClrAttributeUse], boundaries: &mut BTreeSet<String>) -> Vec<Selector> {
    let mut templated = Vec::new();
    let mut silent_methods = BTreeSet::new();
    let mut silent_unknown = false;
    for use_ in attributes {
        let provides_route = use_.implements(ROUTE_TEMPLATE_PROVIDER);
        let provides_methods = use_.implements(HTTP_METHOD_PROVIDER);
        if !provides_route && !provides_methods {
            continue;
        }
        let (methods, unknown) = if provides_methods {
            http_methods(use_)
        } else {
            (BTreeSet::new(), false)
        };
        let template = if provides_route {
            match route_template(use_) {
                Ok(template) => template,
                Err(()) => {
                    boundaries.insert("CUSTOM_ROUTE_TEMPLATE_PROVIDER_REQUIRES_REVIEW".into());
                    None
                }
            }
        } else {
            None
        };
        match template {
            Some(template) => templated.push(Selector {
                template: Some(template),
                methods,
                unknown_methods: unknown,
                attribute: Some(use_.attribute_type.clone()),
            }),
            None => {
                silent_methods.extend(methods);
                silent_unknown |= unknown;
            }
        }
    }
    if templated.is_empty() {
        return vec![Selector {
            template: None,
            methods: silent_methods,
            unknown_methods: silent_unknown,
            attribute: None,
        }];
    }
    for selector in &mut templated {
        selector.methods.extend(silent_methods.iter().cloned());
        selector.unknown_methods |= silent_unknown;
    }
    templated
}

/// `Ok(None)` for a provider without a template, `Err` when the template is not
/// observable from the attribute's constructor (custom providers).
fn route_template(use_: &ClrAttributeUse) -> Result<Option<String>, ()> {
    if use_.is_a(ACCEPT_VERBS) {
        return Ok(use_
            .named_arguments
            .get("Route")
            .and_then(ClrValue::as_string)
            .map(str::to_owned));
    }
    if use_.is_a(ROUTE) || use_.is_a(HTTP_METHOD) {
        let framework = use_.attribute_type.starts_with(MVC);
        return match use_.argument("template") {
            Some(value) => value
                .as_string()
                .map(|value| Some(value.to_owned()))
                .ok_or(()),
            None if framework => Ok(None),
            // A derived attribute without a `template` parameter sets it internally.
            None if use_.constructor_arguments.is_empty() => Ok(None),
            None => Err(()),
        };
    }
    Err(())
}

/// HTTP methods of a method provider and whether they could not be determined.
fn http_methods(use_: &ClrAttributeUse) -> (BTreeSet<String>, bool) {
    if use_.is_a(ACCEPT_VERBS) {
        let mut methods = BTreeSet::new();
        for value in &use_.constructor_arguments {
            match value.strings() {
                Some(values) => {
                    methods.extend(values.into_iter().map(|value| value.to_uppercase()))
                }
                None => return (BTreeSet::new(), true),
            }
        }
        return (methods, false);
    }
    for (name, verb) in VERBS {
        let framework = format!("{MVC}{name}");
        if use_.attribute_type == framework {
            return (BTreeSet::from([verb.to_owned()]), false);
        }
    }
    for (name, verb) in VERBS {
        // A custom attribute derived from a verb attribute inherits its methods.
        if use_
            .attribute_bases
            .iter()
            .any(|base| *base == format!("{MVC}{name}"))
        {
            return (BTreeSet::from([verb.to_owned()]), false);
        }
    }
    (BTreeSet::new(), true)
}

/// ASP.NET Core attribute-route combination: an action template starting with
/// `/` or `~/` overrides the controller route; otherwise templates join with `/`.
fn combine(controller: Option<&str>, action: Option<&str>) -> Option<String> {
    let overrides = |value: &str| value.starts_with('/') || value.starts_with("~/");
    let clean = |value: &str| {
        value
            .trim_start_matches("~/")
            .trim_start_matches('/')
            .trim_end_matches('/')
            .to_owned()
    };
    match (controller, action) {
        (_, Some(action)) if overrides(action) => Some(clean(action)),
        (Some(controller), Some(action)) => {
            let (controller, action) = (clean(controller), clean(action));
            Some(match (controller.is_empty(), action.is_empty()) {
                (true, _) => action,
                (_, true) => controller,
                _ => format!("{controller}/{action}"),
            })
        }
        (Some(controller), None) => Some(clean(controller)),
        (None, Some(action)) => Some(clean(action)),
        (None, None) => None,
    }
}

/// Replace `[controller]`, `[action]` and `[area]`; `[[` and `]]` escape brackets.
/// Unknown tokens need runtime route-value resolution.
fn replace_tokens(
    template: &str,
    controller: &str,
    action: &str,
    area: Option<&str>,
) -> Option<String> {
    let mut output = String::with_capacity(template.len() + 16);
    let mut characters = template.chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            '[' if characters.peek() == Some(&'[') => {
                characters.next();
                output.push('[');
            }
            ']' if characters.peek() == Some(&']') => {
                characters.next();
                output.push(']');
            }
            '[' => {
                let token: String = characters
                    .by_ref()
                    .take_while(|value| *value != ']')
                    .collect();
                match token.to_ascii_lowercase().as_str() {
                    "controller" => output.push_str(controller),
                    "action" => output.push_str(action),
                    "area" => output.push_str(area?),
                    _ => return None,
                }
            }
            ']' => return None,
            other => output.push(other),
        }
    }
    Some(format!("/{output}"))
}

#[derive(Debug, Default)]
struct Versions {
    declared: Vec<String>,
}

fn api_versions(
    type_attributes: &[&ClrAttributeUse],
    action_attributes: &[&ClrAttributeUse],
) -> Versions {
    let collect = |attributes: &[&ClrAttributeUse], names: &[&str]| {
        attributes
            .iter()
            .filter(|use_| names.iter().any(|name| use_.is_a(name)))
            .flat_map(|use_| use_.constructor_arguments.iter().filter_map(version_text))
            .collect::<BTreeSet<_>>()
    };
    let mapped = collect(action_attributes, &MAP_TO_API_VERSION);
    let declared = if mapped.is_empty() {
        let mut values = collect(type_attributes, &API_VERSION);
        values.extend(collect(action_attributes, &API_VERSION));
        values
    } else {
        mapped
    };
    Versions {
        declared: declared.into_iter().collect(),
    }
}

fn version_text(value: &ClrValue) -> Option<String> {
    match value {
        ClrValue::Primitive {
            value: Some(value), ..
        } => Some(value.clone()),
        _ => None,
    }
}

/// Paths with an `{name:apiVersion}` segment yield one candidate per declared
/// version; the segment's text format is runtime configuration.
fn expand_versions(
    paths: BTreeSet<String>,
    versions: &Versions,
    boundary: &mut bool,
) -> (BTreeSet<String>, Value) {
    let mut versioned = BTreeSet::new();
    for path in &paths {
        let Some(start) = path.find('{') else {
            continue;
        };
        let Some(length) = path[start..].find('}') else {
            continue;
        };
        let segment = &path[start..start + length + 1];
        if !segment.ends_with(":apiVersion}") {
            continue;
        }
        *boundary = true;
        for version in &versions.declared {
            versioned.insert(path.replacen(segment, version, 1));
        }
    }
    let value = if versioned.is_empty() {
        Value::Null
    } else {
        json!(versioned)
    };
    (paths, value)
}

fn conditions(
    type_attributes: &[&ClrAttributeUse],
    action_attributes: &[&ClrAttributeUse],
) -> Value {
    let strings = |attributes: &[&ClrAttributeUse], name: &str| -> Option<Vec<String>> {
        let mut values = Vec::new();
        for use_ in attributes.iter().filter(|use_| use_.is_a(name)) {
            for argument in &use_.constructor_arguments {
                values.extend(argument.strings()?);
            }
        }
        Some(values)
    };
    // Action-level content types replace controller-level ones.
    let pick = |name: &str| match strings(action_attributes, name) {
        Some(values) if values.is_empty() => strings(type_attributes, name),
        other => other,
    };
    let all: Vec<&ClrAttributeUse> = type_attributes
        .iter()
        .chain(action_attributes)
        .copied()
        .collect();
    let policies = all
        .iter()
        .filter(|use_| use_.is_a(AUTHORIZE))
        .map(|use_| {
            json!({
                "policy":use_.argument("policy").or_else(|| use_.named_arguments.get("Policy")).and_then(ClrValue::as_string),
                "roles":use_.named_arguments.get("Roles").and_then(ClrValue::as_string),
                "authenticationSchemes":use_.named_arguments.get("AuthenticationSchemes").and_then(ClrValue::as_string),
            })
        })
        .collect::<Vec<_>>();
    json!({
        "consumes":pick(CONSUMES),
        "produces":pick(PRODUCES),
        "authorization":{
            "allowAnonymous":action_attributes.iter().any(|use_| use_.is_a(ALLOW_ANONYMOUS))
                || type_attributes.iter().any(|use_| use_.is_a(ALLOW_ANONYMOUS)),
            "authorize":policies,
        },
    })
}

fn string_attribute(
    attributes: &[&ClrAttributeUse],
    name: &str,
    parameter: &str,
) -> Option<String> {
    attributes
        .iter()
        .find(|use_| use_.is_a(name))
        .and_then(|use_| {
            use_.argument(parameter)
                .or_else(|| use_.constructor_arguments.first())
        })
        .and_then(ClrValue::as_string)
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attribute(
        kind: &str,
        bases: &[&str],
        interfaces: &[&str],
        arguments: &[(&str, &str)],
    ) -> Value {
        json!({
            "attributeType":kind,
            "attributeBases":bases,
            "attributeInterfaces":interfaces,
            "constructorArguments":arguments.iter().map(|(_, value)| json!({"kind":"PRIMITIVE","type":"string","value":value})).collect::<Vec<_>>(),
            "namedArguments":{},
            "constructorParameters":arguments.iter().map(|(name, _)| name).collect::<Vec<_>>(),
        })
    }

    fn route(template: &str) -> Value {
        attribute(
            ROUTE,
            &[],
            &[ROUTE_TEMPLATE_PROVIDER],
            &[("template", template)],
        )
    }

    fn verb(name: &str, template: Option<&str>) -> Value {
        let arguments: Vec<(&str, &str)> = template
            .map(|value| ("template", value))
            .into_iter()
            .collect();
        attribute(
            &format!("{MVC}{name}"),
            &[HTTP_METHOD],
            &[HTTP_METHOD_PROVIDER, ROUTE_TEMPLATE_PROVIDER],
            &arguments,
        )
    }

    fn api_controller() -> Value {
        attribute(
            "class:Microsoft.AspNetCore.Mvc.ApiControllerAttribute",
            &[CONTROLLER],
            &[],
            &[],
        )
    }

    fn facts(
        type_name: &str,
        accessibility: &str,
        method: &str,
        type_attributes: Vec<Value>,
        method_attributes: Vec<Value>,
    ) -> ClrAttributeFacts {
        serde_json::from_value(json!({
            "schema":clew_facts::CLR_ATTRIBUTE_SCHEMA,
            "authority":clew_facts::CLR_ATTRIBUTE_AUTHORITY,
            "declaration":format!("method:class:Orders.{type_name}#{method}()V"),
            "method":{"name":method,"accessibility":"PUBLIC","isStatic":false,"isAbstract":false,
                "isGeneric":false,"isOverride":false,"attributes":method_attributes},
            "containingType":{"identity":format!("class:Orders.{type_name}"),"name":type_name,
                "accessibility":accessibility,"isAbstract":false,"isGeneric":false,"isNested":false,
                "attributes":type_attributes,"baseTypes":["class:Microsoft.AspNetCore.Mvc.ControllerBase"]},
            "overriddenAttributes":[],
            "boundaries":[],
            "coverage":{"status":"COMPLETE","scope":clew_facts::CLR_ATTRIBUTE_SCOPE},
        }))
        .unwrap()
    }

    fn single(metadata: &AspNetCoreMetadata) -> &Value {
        assert_eq!(metadata.entries.len(), 1, "{metadata:?}");
        &metadata.entries[0].trigger
    }

    #[test]
    fn silent_verb_constrains_a_templated_route_attribute() {
        let metadata = analyze(&facts(
            "KaskoController",
            "INTERNAL",
            "Save",
            vec![api_controller(), route("api/kasko")],
            vec![verb("HttpPostAttribute", None), route("save")],
        ))
        .unwrap();
        let trigger = single(&metadata);
        assert_eq!(trigger["paths"], json!(["/api/kasko/save"]));
        assert_eq!(trigger["methods"], json!(["POST"]));
        assert_eq!(trigger["pathResolution"], "DERIVED");
        assert!(
            metadata
                .boundaries
                .contains(&"CONTROLLER_DISCOVERY_REQUIRES_CUSTOM_FEATURE_PROVIDER".to_owned())
        );
    }

    #[test]
    fn tokens_overrides_and_multiple_selectors_follow_the_application_model() {
        let metadata = analyze(&facts(
            "OrdersController",
            "PUBLIC",
            "GetAsync",
            vec![api_controller(), route("api/[controller]")],
            vec![
                verb("HttpGetAttribute", Some("{id:int}")),
                verb("HttpGetAttribute", Some("~/legacy/[action]")),
            ],
        ))
        .unwrap();
        let paths: BTreeSet<_> = metadata
            .entries
            .iter()
            .flat_map(|entry| entry.trigger["paths"].as_array().unwrap().clone())
            .map(|path| path.as_str().unwrap().to_owned())
            .collect();
        assert_eq!(
            paths,
            BTreeSet::from(["/api/Orders/{id:int}".to_owned(), "/legacy/Get".to_owned()])
        );
        assert!(
            metadata
                .boundaries
                .contains(&"ACTION_NAME_ASYNC_SUFFIX_DEFAULT_ASSUMED".to_owned())
        );
        assert!(
            metadata
                .entries
                .iter()
                .all(|entry| entry.trigger["methods"] == json!(["GET"]))
        );
    }

    #[test]
    fn controller_route_without_action_template_accepts_any_method() {
        let metadata = analyze(&facts(
            "HealthController",
            "PUBLIC",
            "Get",
            vec![route("health")],
            vec![],
        ))
        .unwrap();
        let trigger = single(&metadata);
        assert_eq!(trigger["paths"], json!(["/health"]));
        assert_eq!(trigger["methods"], json!(["ANY"]));
    }

    #[test]
    fn area_and_unknown_tokens_are_resolved_or_bounded() {
        let area = attribute(AREA, &[], &[], &[("areaName", "admin")]);
        let metadata = analyze(&facts(
            "AccountsController",
            "PUBLIC",
            "List",
            vec![area, route("[area]/[controller]/[action]")],
            vec![verb("HttpGetAttribute", None)],
        ))
        .unwrap();
        assert_eq!(single(&metadata)["paths"], json!(["/admin/Accounts/List"]));
        let metadata = analyze(&facts(
            "AccountsController",
            "PUBLIC",
            "List",
            vec![route("[tenant]/users")],
            vec![],
        ))
        .unwrap();
        let trigger = single(&metadata);
        assert!(trigger["paths"].is_null());
        assert_eq!(
            trigger["pathResolution"],
            "REQUIRES_RUNTIME_ROUTE_RESOLUTION"
        );
    }

    #[test]
    fn discovery_conventions_exclude_non_actions_and_non_controllers() {
        let non_action = attribute(NON_ACTION, &[], &[], &[]);
        assert!(
            analyze(&facts(
                "OrdersController",
                "PUBLIC",
                "Helper",
                vec![route("api")],
                vec![non_action]
            ))
            .unwrap()
            .entries
            .is_empty()
        );
        let non_controller = attribute(NON_CONTROLLER, &[], &[], &[]);
        assert!(
            analyze(&facts(
                "OrdersController",
                "PUBLIC",
                "Get",
                vec![non_controller, route("api")],
                vec![]
            ))
            .unwrap()
            .entries
            .is_empty()
        );
        assert!(
            analyze(&facts("OrderService", "PUBLIC", "Get", vec![], vec![]))
                .unwrap()
                .entries
                .is_empty()
        );
        let conventional =
            analyze(&facts("HomeController", "PUBLIC", "Index", vec![], vec![])).unwrap();
        assert!(single(&conventional)["paths"].is_null());
        assert!(
            conventional
                .boundaries
                .contains(&"CONVENTIONAL_ROUTING_REQUIRES_RUNTIME_CONFIGURATION".to_owned())
        );
    }

    #[test]
    fn api_version_segments_yield_candidates_with_a_boundary() {
        let version = attribute(
            "class:Asp.Versioning.ApiVersionAttribute",
            &[],
            &[],
            &[("version", "2.0")],
        );
        let metadata = analyze(&facts(
            "QuotesController",
            "PUBLIC",
            "Get",
            vec![version, route("api/v{version:apiVersion}/quotes")],
            vec![verb("HttpGetAttribute", None)],
        ))
        .unwrap();
        let trigger = single(&metadata);
        assert_eq!(trigger["apiVersions"], json!(["2.0"]));
        assert_eq!(trigger["versionedPaths"], json!(["/api/v2.0/quotes"]));
        assert!(
            metadata
                .boundaries
                .contains(&"API_VERSION_ROUTE_SEGMENT_FORMAT_RUNTIME_CONFIGURED".to_owned())
        );
    }

    #[test]
    fn invalid_input_authority_is_rejected() {
        let mut input = facts("OrdersController", "PUBLIC", "Get", vec![], vec![]);
        input.authority = "SOURCE_NAMES".into();
        assert!(analyze(&input).is_err());
    }
}
