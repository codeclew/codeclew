use crate::error::{ClewError, ErrorCode};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const TRANSFORM_KIND: &str = "PROPAGATE_TYPED_FIELDS";

pub fn expand_transient_transform(
    plan: &mut Value,
    context: &Value,
    evidence: &Value,
) -> Result<(), ClewError> {
    let Some(transform) = plan
        .get("transform")
        .or_else(|| plan.get("transformation"))
        .cloned()
    else {
        return Ok(());
    };
    if plan.get("operations").is_some() || plan.get("edits").is_some() {
        return Err(invalid(
            "a transient transform cannot be mixed with low-level operations",
        ));
    }
    if transform["kind"].as_str() != Some(TRANSFORM_KIND) {
        return Err(invalid(format!(
            "unsupported transient transform {}; expected {TRANSFORM_KIND}",
            transform["kind"].as_str().unwrap_or("<missing>")
        )));
    }
    require_planning_evidence(evidence)?;

    let surfaces = role_surfaces(context)?;
    let workflow = surfaces["WORKFLOW"];
    let intermediary = surfaces["INTERMEDIARY"];
    let output = surfaces["OUTPUT_CONTRACT"];
    let data_source = surfaces["DATA_SOURCE"];
    verify_resolved_path(evidence, workflow, intermediary, output, data_source)?;

    let contract = unique_item(context, "contracts")?;
    let test = unique_item(context, "tests")?;
    let fields = requested_fields(&transform, context)?;
    let names = transform
        .get("names")
        .ok_or_else(|| invalid("transform needs names"))?;
    let interface_name = identifier(required_string(names, "newContract")?, "names.newContract")?;
    let record_name = identifier(
        required_string(names, "newProjection")?,
        "names.newProjection",
    )?;
    if interface_name == record_name {
        return Err(invalid(
            "names.newContract and names.newProjection must be distinct",
        ));
    }
    let current_contract = required_string(contract, "name")?;
    let existing_names = context_names(context);
    for new_name in [&interface_name, &record_name] {
        if existing_names.contains(new_name.as_str()) {
            return Err(invalid(format!(
                "new type {new_name} collides with an emitted declaration"
            )));
        }
    }
    let contract_source = required_string(contract, "sourceText")?;
    let workflow_source = required_string(workflow, "sourceText")?;
    let intermediary_source = required_string(intermediary, "sourceText")?;
    let output_source = required_string(output, "sourceText")?;
    let data_source_source = required_string(data_source, "sourceText")?;
    let test_source = required_string(test, "sourceText")?;
    let old_data_source_name = required_string(data_source, "name")?;

    let contract_file = required_string(contract, "file")?;
    let contract_parent = Path::new(&contract_file)
        .parent()
        .ok_or_else(|| invalid("contract file has no parent directory"))?;
    let created_file = contract_parent
        .join(format!("{interface_name}.kt"))
        .to_string_lossy()
        .replace('\\', "/");
    let package = kotlin_package(&contract_file)?;
    let imports = transform["names"]["imports"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| invalid("names.imports must contain strings"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let created_source =
        render_contract_source(&package, &imports, &interface_name, &record_name, &fields);

    let (header_old, header_new) = contract_header_rewrite(&contract_source, &interface_name)?;
    let old_collection = infer_collection(&workflow_source, &old_data_source_name)?;
    let old_item = infer_loop_item(&workflow_source, &old_collection)?;
    let (query_old, query_new, identity_field) =
        projection_query_rewrite(&data_source_source, &package, &record_name, &fields)?;
    let payload_parameter = parameter_for_type(&intermediary_source, &current_contract)?;
    let identity_parameter = infer_identity_parameter(&workflow_source, &old_item)?;
    let return_rewrite =
        return_type_rewrite(&data_source_source, &old_data_source_name, &record_name)?;
    let test_old = format!("{payload_parameter} = anyOrNull(),");
    let test_occurrence = 1_u64;
    if !test_source.contains(&test_old) {
        return Err(incomplete(format!(
            "test payload matcher {test_old:?} is absent"
        )));
    }
    let test_expected = infer_test_expected(&test_source, &test_old)?;
    let assertions = fields
        .iter()
        .map(|field| format!("{} == {test_expected}.{}", field.name, field.name))
        .collect::<Vec<_>>()
        .join(" && ");
    let test_new = format!("{payload_parameter} = argThat {{ {assertions} }},");

    let workflow_substitutions = workflow_substitutions(
        &workflow_source,
        &old_data_source_name,
        &old_collection,
        &old_item,
        &identity_field,
        &identity_parameter,
        &payload_parameter,
    )?;

    plan["operations"] = Value::Array(vec![
        json!({
            "kind":"CREATE_FILE",
            "path":created_file,
            "kotlinLines":created_source.lines().collect::<Vec<_>>()
        }),
        rewrite(contract, vec![substitution(&header_old, &header_new, 1)])?,
        rewrite(
            intermediary,
            vec![substitution(
                &current_contract,
                &interface_name,
                intermediary_source.matches(&current_contract).count(),
            )],
        )?,
        rewrite(
            output,
            vec![substitution(
                &current_contract,
                &interface_name,
                output_source.matches(&current_contract).count(),
            )],
        )?,
        rewrite(
            data_source,
            vec![
                substitution_occurrence(&query_old, &query_new, 1),
                substitution(&return_rewrite.0, &return_rewrite.1, 1),
            ],
        )?,
        rewrite(workflow, workflow_substitutions)?,
        json!({
            "kind":"REWRITE_DECLARATION",
            "target":{"targetId":target_id(test)?},
            "old":test_old,
            "new":test_new,
            "occurrence":test_occurrence
        }),
    ]);
    plan["expandedTransform"] = json!({
        "kind":TRANSFORM_KIND,
        "roles":["WORKFLOW","INTERMEDIARY","OUTPUT_CONTRACT","DATA_SOURCE"],
        "fields":fields.iter().map(|field| field.name.as_str()).collect::<Vec<_>>()
    });
    Ok(())
}

#[derive(Clone, Debug)]
struct Field {
    name: String,
    field_type: String,
    source: String,
}

fn requested_fields(transform: &Value, context: &Value) -> Result<Vec<Field>, ClewError> {
    let requested = transform["fields"]
        .as_array()
        .ok_or_else(|| invalid("transform needs a fields array"))?;
    if requested.is_empty() {
        return Err(invalid("transform fields cannot be empty"));
    }
    let available = context["projectionFields"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|field| field["name"].as_str().map(|name| (name, field)))
        .collect::<BTreeMap<_, _>>();
    let mut fields = Vec::new();
    for value in requested {
        let name = identifier(
            value
                .as_str()
                .ok_or_else(|| invalid("transform fields must be strings"))?,
            "field",
        )?;
        let field = available.get(name.as_str()).ok_or_else(|| {
            incomplete(format!("requested field {name} has no projection evidence"))
        })?;
        fields.push(Field {
            name,
            field_type: required_string(field, "type")?,
            source: required_string(field, "source")?,
        });
    }
    Ok(fields)
}

fn role_surfaces(context: &Value) -> Result<BTreeMap<&'static str, &Value>, ClewError> {
    let mut result = BTreeMap::new();
    for role in ["WORKFLOW", "INTERMEDIARY", "OUTPUT_CONTRACT", "DATA_SOURCE"] {
        let matches = context["editSurfaces"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|surface| surface["role"].as_str() == Some(role))
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            return Err(incomplete(format!(
                "transient transform needs exactly one {role} surface; found {}",
                matches.len()
            )));
        }
        result.insert(role, matches[0]);
    }
    Ok(result)
}

fn unique_item<'a>(context: &'a Value, section: &str) -> Result<&'a Value, ClewError> {
    let items = context[section]
        .as_array()
        .ok_or_else(|| incomplete(format!("context has no {section} array")))?;
    if items.len() != 1 {
        return Err(incomplete(format!(
            "transient transform needs exactly one {section} item; found {}",
            items.len()
        )));
    }
    Ok(&items[0])
}

fn require_planning_evidence(evidence: &Value) -> Result<(), ClewError> {
    for section in ["resolutions", "threads"] {
        if evidence[section].as_array().is_none_or(Vec::is_empty) {
            return Err(incomplete(format!(
                "transient transform needs full {section} evidence"
            )));
        }
    }
    Ok(())
}

fn verify_resolved_path(
    evidence: &Value,
    workflow: &Value,
    intermediary: &Value,
    output: &Value,
    data_source: &Value,
) -> Result<(), ClewError> {
    let workflow_name = required_string(workflow, "name")?;
    let intermediary_name = required_string(intermediary, "name")?;
    let output_name = required_string(output, "name")?;
    let data_source_name = required_string(data_source, "name")?;
    let resolutions = evidence["resolutions"].as_array().unwrap();
    let calls_from = |owner: &str, callee: &str| {
        resolutions.iter().any(|resolution| {
            let declaration_name = resolution
                .pointer("/declaration/name")
                .and_then(Value::as_str)
                .or_else(|| {
                    resolution
                        .pointer("/declaration/symbolIdentity/declarationName")
                        .and_then(Value::as_str)
                });
            declaration_name == Some(owner)
                && resolution["resolvedCalls"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|call| call["symbol"].as_str())
                    .any(|symbol| symbol_tail_matches(symbol, callee))
        })
    };
    for (owner, callee) in [
        (workflow_name.as_str(), data_source_name.as_str()),
        (workflow_name.as_str(), intermediary_name.as_str()),
        (intermediary_name.as_str(), output_name.as_str()),
    ] {
        if !calls_from(owner, callee) {
            return Err(incomplete(format!(
                "full evidence has no resolved {owner} -> {callee} edge"
            )));
        }
    }
    Ok(())
}

fn symbol_tail_matches(symbol: &str, name: &str) -> bool {
    symbol
        .rsplit(['.', '/'])
        .next()
        .is_some_and(|tail| tail == name)
}

fn render_contract_source(
    package: &str,
    imports: &[String],
    interface_name: &str,
    record_name: &str,
    fields: &[Field],
) -> String {
    let import_block = if imports.is_empty() {
        String::new()
    } else {
        format!(
            "\n{}\n",
            imports
                .iter()
                .map(|import| format!("import {import}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    };
    let interface_fields = fields
        .iter()
        .map(|field| format!("    val {}: {}", field.name, field.field_type))
        .collect::<Vec<_>>()
        .join("\n");
    let record_fields = fields
        .iter()
        .map(|field| format!("    override val {}: {},", field.name, field.field_type))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "package {package}\n{import_block}\ninterface {interface_name} {{\n{interface_fields}\n}}\n\ndata class {record_name}(\n{record_fields}\n) : {interface_name}\n"
    )
}

fn contract_header_rewrite(
    source: &str,
    interface_name: &str,
) -> Result<(String, String), ClewError> {
    let old = "\n) {\n";
    if source.matches(old).count() != 1 {
        return Err(incomplete(
            "canonical contract header is not a unique class-with-body shape",
        ));
    }
    Ok((old.to_owned(), format!("\n) : {interface_name} {{\n")))
}

fn projection_query_rewrite(
    source: &str,
    package: &str,
    record_name: &str,
    fields: &[Field],
) -> Result<(String, String, String), ClewError> {
    let lower = source.to_lowercase();
    let select = lower
        .find("select ")
        .ok_or_else(|| incomplete("data source has no SELECT clause"))?
        + "select ".len();
    let from = lower[select..]
        .find(" from ")
        .map(|index| select + index)
        .ok_or_else(|| incomplete("data source SELECT has no FROM clause"))?;
    let old = source[select..from].to_owned();
    let alias = old
        .trim()
        .split_once('.')
        .map(|(alias, _)| alias.trim())
        .filter(|alias| !alias.is_empty())
        .ok_or_else(|| incomplete("data source projection has no resolvable alias"))?;
    let identity_field = old
        .trim()
        .split_once('.')
        .map(|(_, field)| field.trim())
        .filter(|field| fields.iter().any(|candidate| candidate.name == *field))
        .map(str::to_owned)
        .ok_or_else(|| incomplete("data source scalar projection is not a requested field"))?;
    let expressions = fields
        .iter()
        .map(|field| {
            let source_field = field.source.rsplit('.').next().unwrap_or(&field.name);
            format!("{alias}.{source_field}")
        })
        .collect::<Vec<_>>()
        .join(", ");
    Ok((
        old,
        format!("new {package}.{record_name}({expressions})"),
        identity_field,
    ))
}

fn parameter_for_type(source: &str, type_name: &str) -> Result<String, ClewError> {
    let signature = source
        .split_once('{')
        .map(|(signature, _)| signature)
        .unwrap_or(source);
    let parameter_list = signature
        .split_once('(')
        .and_then(|(_, tail)| tail.rsplit_once(')').map(|(parameters, _)| parameters))
        .ok_or_else(|| incomplete("intermediary declaration has no parameter list"))?;
    let parameters = parameter_list
        .split(',')
        .filter_map(|parameter| {
            let (name, type_and_default) = parameter.trim().split_once(':')?;
            let candidate = type_and_default.trim();
            let type_matches = candidate == type_name
                || candidate.starts_with(&format!("{type_name}?"))
                || candidate.starts_with(&format!("{type_name} "));
            type_matches.then(|| name.trim().to_owned())
        })
        .collect::<BTreeSet<_>>();
    if parameters.len() != 1 {
        return Err(incomplete(format!(
            "intermediary payload parameter for {type_name} is ambiguous; found {parameters:?}"
        )));
    }
    Ok(parameters.into_iter().next().unwrap())
}

fn infer_identity_parameter(source: &str, loop_item: &str) -> Result<String, ClewError> {
    let parameters = source
        .split([',', '\n', '('])
        .filter_map(|fragment| {
            let (parameter, value) = fragment.trim().split_once(" = ")?;
            (value == loop_item).then(|| parameter.trim().to_owned())
        })
        .filter(|parameter| identifier(parameter.clone(), "identity parameter").is_ok())
        .collect::<BTreeSet<_>>();
    if parameters.len() != 1 {
        return Err(incomplete(format!(
            "workflow identity argument for {loop_item} is ambiguous; found {parameters:?}"
        )));
    }
    Ok(parameters.into_iter().next().unwrap())
}

fn infer_test_expected(source: &str, matcher: &str) -> Result<String, ClewError> {
    let matcher_offset = source
        .find(matcher)
        .ok_or_else(|| incomplete("test payload matcher is absent"))?;
    let prefix = &source[..matcher_offset];
    let lambda = prefix
        .rfind(".forEach {")
        .map(|offset| &prefix[offset..])
        .ok_or_else(|| incomplete("test matcher is not inside an emitted forEach binding"))?;
    let binding = lambda
        .split_once('{')
        .and_then(|(_, tail)| tail.split_once("->").map(|(binding, _)| binding.trim()))
        .ok_or_else(|| incomplete("test forEach binding is incomplete"))?;
    identifier(binding, "test expected binding")
}

fn return_type_rewrite(
    source: &str,
    method_name: &str,
    record_name: &str,
) -> Result<(String, String), ClewError> {
    let method = source
        .find(method_name)
        .ok_or_else(|| incomplete("data source method name is absent from its source"))?;
    let tail = &source[method..];
    let marker = "): List<";
    let start = tail
        .find(marker)
        .map(|index| method + index)
        .ok_or_else(|| incomplete("data source return type is not List<T>"))?;
    let end = source[start + marker.len()..]
        .find('>')
        .map(|index| start + marker.len() + index + 1)
        .ok_or_else(|| incomplete("data source List<T> return type is incomplete"))?;
    Ok((
        source[start..end].to_owned(),
        format!("): List<{record_name}>"),
    ))
}

fn infer_collection(source: &str, method_name: &str) -> Result<String, ClewError> {
    let names = source
        .lines()
        .filter(|line| line.contains(method_name))
        .filter_map(|line| {
            line.trim()
                .strip_prefix("val ")?
                .split_once('=')
                .map(|(name, _)| name.trim().to_owned())
        })
        .collect::<BTreeSet<_>>();
    if names.len() != 1 {
        return Err(incomplete(format!(
            "workflow data-source binding is ambiguous; found {names:?}"
        )));
    }
    Ok(names.into_iter().next().unwrap())
}

fn infer_loop_item(source: &str, collection: &str) -> Result<String, ClewError> {
    let marker = format!("{collection}.forEach {{");
    let items = source
        .match_indices(&marker)
        .filter_map(|(index, _)| {
            source[index + marker.len()..]
                .split_once("->")
                .map(|(item, _)| item.trim().to_owned())
        })
        .collect::<BTreeSet<_>>();
    if items.len() != 1 {
        return Err(incomplete(format!(
            "workflow loop binding is ambiguous; found {items:?}"
        )));
    }
    Ok(items.into_iter().next().unwrap())
}

#[allow(clippy::too_many_arguments)]
fn workflow_substitutions(
    source: &str,
    old_method: &str,
    old_collection: &str,
    old_item: &str,
    identity_field: &str,
    identity_parameter: &str,
    payload_parameter: &str,
) -> Result<Vec<Value>, ClewError> {
    let specs = vec![
        (
            format!("{identity_parameter} = {old_item},"),
            format!(
                "{identity_parameter} = {old_item}.{identity_field},\n                        {payload_parameter} = {old_item},"
            ),
        ),
        (
            format!("({old_item})"),
            format!("({old_item}.{identity_field})"),
        ),
    ];
    if !source.contains(old_method) || !source.contains(old_collection) {
        return Err(incomplete(
            "workflow no longer contains the resolved data-source binding",
        ));
    }
    specs
        .into_iter()
        .map(|(old, new)| {
            let count = source.matches(&old).count();
            if count == 0 {
                return Err(incomplete(format!(
                    "workflow transform cannot find required fragment {old:?}"
                )));
            }
            Ok(substitution(&old, &new, count))
        })
        .collect()
}

fn context_names(context: &Value) -> BTreeSet<&str> {
    ["editSurfaces", "contracts", "tests"]
        .into_iter()
        .flat_map(|section| {
            context[section]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|item| item["name"].as_str())
        })
        .collect()
}

fn rewrite(target: &Value, substitutions: Vec<Value>) -> Result<Value, ClewError> {
    Ok(json!({
        "kind":"REWRITE_DECLARATION",
        "target":{"targetId":target_id(target)?},
        "substitutions":substitutions
    }))
}

fn substitution(old: &str, new: &str, occurrences: usize) -> Value {
    json!({"old":old,"new":new,"occurrences":occurrences})
}

fn substitution_occurrence(old: &str, new: &str, occurrence: usize) -> Value {
    json!({"old":old,"new":new,"occurrence":occurrence})
}

fn target_id(item: &Value) -> Result<String, ClewError> {
    item["declarationTargetId"]
        .as_str()
        .map(str::to_owned)
        .or_else(|| {
            item.pointer("/declarationTarget/anchorId")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .ok_or_else(|| incomplete("context item has no declaration target"))
}

fn kotlin_package(file: &str) -> Result<String, ClewError> {
    file.split_once("/kotlin/")
        .and_then(|(_, relative)| relative.rsplit_once('/'))
        .map(|(package, _)| package.replace('/', "."))
        .filter(|package| !package.is_empty())
        .ok_or_else(|| invalid("contract file is not under a Kotlin source root"))
}

fn required_string(value: &Value, key: &str) -> Result<String, ClewError> {
    value[key]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| invalid(format!("missing string field {key}")))
}

fn identifier(value: impl Into<String>, label: &str) -> Result<String, ClewError> {
    let value = value.into();
    if value.is_empty()
        || value
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_digit())
        || !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
    {
        return Err(invalid(format!("{label} must be a Kotlin identifier")));
    }
    Ok(value)
}

fn invalid(message: impl Into<String>) -> ClewError {
    ClewError::new(ErrorCode::InvalidInput, message)
}

fn incomplete(message: impl Into<String>) -> ClewError {
    ClewError::new(ErrorCode::IncompleteSemanticAnalysis, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> Value {
        json!({
            "editSurfaces":[
                {
                    "declarationTargetId":"S1","name":"reconcile","role":"WORKFLOW",
                    "file":"src/main/kotlin/com/acme/ReconcileService.kt",
                    "sourceText":"fun reconcile() {\n    val selectedKeys = repository.findKeys(batch)\n    selectedKeys.forEach { key ->\n        notifier.notify(identity = key,\n            audit = audit(key))\n    }\n    total += selectedKeys.size\n}"
                },
                {
                    "declarationTargetId":"S2","name":"notify","role":"INTERMEDIARY",
                    "file":"src/main/kotlin/com/acme/Notifier.kt",
                    "sourceText":"fun notify(identity: String?, payload: Canonical? = null) { Envelope(payload) }"
                },
                {
                    "declarationTargetId":"S3","name":"Envelope","role":"OUTPUT_CONTRACT",
                    "file":"src/main/kotlin/com/acme/Envelope.kt",
                    "sourceText":"data class Envelope(val payload: Canonical?)"
                },
                {
                    "declarationTargetId":"S4","name":"findKeys","role":"DATA_SOURCE",
                    "file":"src/main/kotlin/com/acme/Repository.kt",
                    "sourceText":"@Query(\"SELECT r.key FROM Record r WHERE r.key IN :keys\")\nfun findKeys(keys: List<String>): List<String>"
                }
            ],
            "contracts":[{
                "declarationTargetId":"C1","name":"Canonical",
                "file":"src/main/kotlin/com/acme/Canonical.kt",
                "sourceText":"data class Canonical(\n    val key: String,\n    val label: String,\n) {\n    fun stable() = key\n}"
            }],
            "tests":[{
                "declarationTargetId":"T1","name":"reconcile test",
                "file":"src/test/kotlin/com/acme/ReconcileTest.kt",
                "sourceText":"records.forEach { expected ->\n    verify { notifier.notify(identity = expected.key, payload = anyOrNull(), audit = any()) }\n}"
            }],
            "projectionFields":[
                {"name":"key","type":"String","source":"Record.key","nullable":false},
                {"name":"label","type":"String?","source":"Record.label","nullable":true}
            ]
        })
    }

    fn evidence() -> Value {
        json!({
            "threads":[{}],
            "resolutions":[
                {
                    "declaration":{"name":"reconcile"},
                    "resolvedCalls":[
                        {"symbol":"com/acme/Repository.findKeys"},
                        {"symbol":"com/acme/Notifier.notify"}
                    ]
                },
                {
                    "declaration":{"name":"notify"},
                    "resolvedCalls":[{"symbol":"com/acme/Envelope.Envelope"}]
                }
            ]
        })
    }

    fn compact_plan() -> Value {
        json!({
            "schema":"semantic-task-goal/0.4",
            "transform":{
                "kind":"PROPAGATE_TYPED_FIELDS",
                "fields":["key","label"],
                "names":{
                    "newContract":"ChangePayload",
                    "newProjection":"SelectedRecord",
                    "imports":[]
                }
            }
        })
    }

    #[test]
    fn expands_compact_goal_from_roles_and_resolved_edges() {
        let mut plan = compact_plan();
        assert!(serde_json::to_vec(&plan).unwrap().len() < 700);

        expand_transient_transform(&mut plan, &context(), &evidence()).unwrap();

        let operations = plan["operations"].as_array().unwrap();
        assert_eq!(operations.len(), 7);
        assert_eq!(operations[0]["kind"], "CREATE_FILE");
        let created = operations[0]["kotlinLines"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(created.contains("val label: String?"));
        assert!(created.contains("data class SelectedRecord"));
        assert_eq!(plan["expandedTransform"]["kind"], TRANSFORM_KIND);
    }

    #[test]
    fn rejects_a_transform_without_full_resolved_path() {
        let mut plan = compact_plan();
        let mut incomplete_evidence = evidence();
        incomplete_evidence["resolutions"][0]["resolvedCalls"] = json!([]);

        let error =
            expand_transient_transform(&mut plan, &context(), &incomplete_evidence).unwrap_err();

        assert_eq!(error.code, ErrorCode::IncompleteSemanticAnalysis);
        assert!(error.message.contains("resolved reconcile -> findKeys"));
    }

    #[test]
    fn rejects_new_type_names_that_collide_with_emitted_declarations() {
        let mut plan = compact_plan();
        plan["transform"]["names"]["newContract"] = json!("Canonical");

        let error = expand_transient_transform(&mut plan, &context(), &evidence()).unwrap_err();

        assert_eq!(error.code, ErrorCode::InvalidInput);
        assert!(error.message.contains("collides"));
    }
}
