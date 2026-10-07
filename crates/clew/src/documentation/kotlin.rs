//! Project retained K2 descriptors and compiler-linked PSI flow into documentation.
use super::local_cfg::{LOCAL_CFG_BOUNDARY_EVIDENCE_SCHEMA, LocalCfgBoundaryEvidence};
use super::{digest, invalid};
use crate::error::ClewError;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

const DOCUMENTATION_CALL_SCHEMA: &str = "codeclew-kotlin-documentation-call/1.0";
const DOCUMENTATION_LOCAL_CFG_SCHEMA: &str = "codeclew-kotlin-documentation-local-cfg/1.0";

type CallOccurrence = (String, String, u64, u64);

#[derive(Clone)]
struct FunctionDescriptor {
    symbol: String,
    compiler_callable_id: String,
    jvm_descriptor: String,
    start: u64,
    end: u64,
    binding: String,
}

struct ProjectedCall {
    occurrence: CallOccurrence,
    fact: Value,
    binding: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ProjectedLocalCfg {
    schema: &'static str,
    kind: &'static str,
    graph: crate::thread_flow_cfg::LocalCfgPayload,
    owner_symbol_identity: String,
    file: String,
    scope: String,
    owner_start: u64,
    owner_end: u64,
    graph_evidence_binding: String,
    descriptor_evidence_binding: String,
}

// Keep independent owner, source and provenance pins explicit at this validation boundary.
#[allow(clippy::too_many_arguments)]
fn local_cfg_boundary(
    scope: String,
    owner: Option<String>,
    file: Option<String>,
    graph_name: Option<String>,
    code: impl Into<String>,
    binding: String,
    provider: &str,
    raw_row_hash: Option<String>,
) -> Result<(Value, String), ClewError> {
    let fact = serde_json::to_value(LocalCfgBoundaryEvidence {
        schema: LOCAL_CFG_BOUNDARY_EVIDENCE_SCHEMA.into(),
        kind: "LOCAL_CFG_BOUNDARY".into(),
        scope,
        owner_symbol_identity: owner,
        file,
        compiler_graph_name: graph_name,
        code: code.into(),
        provider: provider.into(),
        evidence_binding: binding.clone(),
        descriptor_evidence_binding: None,
        raw_row_hash,
    })
    .map_err(|_| invalid("Kotlin local CFG boundary cannot be serialized"))?;
    Ok((fact, binding))
}

fn fact_scope(fact: &Value) -> String {
    fact["scope"]["compilation"]
        .as_str()
        .or_else(|| fact["scope"].as_str())
        .unwrap_or_default()
        .to_owned()
}

fn without_capture_scope(fact: &Value) -> Value {
    let mut fact = fact.clone();
    if let Some(object) = fact.as_object_mut() {
        object.remove("scope");
    }
    fact
}

fn occurrence(fact: &Value) -> Option<CallOccurrence> {
    Some((
        fact_scope(fact),
        fact["file"].as_str()?.to_owned(),
        fact["start"].as_u64()?,
        fact["end"].as_u64()?,
    ))
}

fn is_target_resolution_boundary(fact: &Value) -> bool {
    let stage = fact["stage"].as_str().unwrap_or_default();
    let code = fact["code"].as_str().unwrap_or_default();
    matches!(
        stage,
        "TARGET_IDENTITY"
            | "CALL_RESOLUTION"
            | "CONSTRUCTOR_RESOLUTION"
            | "TARGET_RESOLUTION"
            | "RELATION_RESOLUTION"
    ) || (stage == "REFERENCE" && code == "UNRESOLVED_CALLABLE_TARGET")
        || (stage == "NORMALIZE"
            && code == "REFERENCE_TO_QUARANTINED_DESCRIPTOR"
            && fact["relationKind"] == "CALLS")
}

fn boundary_applies_to_call(boundary: &Value, relation: &Value) -> bool {
    if !is_target_resolution_boundary(boundary) {
        return false;
    }
    let boundary_scope = fact_scope(boundary);
    if !boundary_scope.is_empty() && boundary_scope != fact_scope(relation) {
        return false;
    }
    if boundary["relationKind"]
        .as_str()
        .is_some_and(|kind| kind != "CALLS")
    {
        return false;
    }
    if boundary["owner"]
        .as_str()
        .is_some_and(|owner| relation["owner"].as_str() != Some(owner))
    {
        return false;
    }
    if let Some(target) = boundary["target"].as_str()
        && relation["target"].as_str() != Some(target)
        && relation["targetCompilerCallableId"].as_str() != Some(target)
    {
        return false;
    }
    if let Some(file) = boundary["file"].as_str()
        && relation["file"].as_str() != Some(file)
    {
        return false;
    }
    if let (Some(start), Some(end)) = (boundary["start"].as_u64(), boundary["end"].as_u64())
        && (relation["start"].as_u64() != Some(start) || relation["end"].as_u64() != Some(end))
    {
        return false;
    }
    true
}

fn exact_call_fact(
    relation: &Value,
    owner: &FunctionDescriptor,
    binding: &str,
) -> Result<Value, ClewError> {
    let mut fact = json!({
        "schema":DOCUMENTATION_CALL_SCHEMA,
        "kind":"RELATION",
        "relationKind":"CALLS",
        "sourceIdentity":owner.symbol,
        "targetIdentity":relation["target"],
        "resolution":"COMPILER_EXACT",
        "provider":"K2_FIR",
        "compilerResolution":"PROVEN",
        "compilerSchema":"declaration-relation/0.1",
        "sourceCompilerCallableId":owner.compiler_callable_id,
        "sourceJvmDescriptor":owner.jvm_descriptor,
        "targetCompilerCallableId":relation["targetCompilerCallableId"],
        "targetJvmDescriptor":relation["targetJvmDescriptor"],
        "file":relation["file"],
        "byteStart":relation["start"],
        "byteEnd":relation["end"],
        "scope":relation["scope"],
        "sourceProvenance":relation["sourceProvenance"],
        "evidenceBinding":binding
    });
    if let (Some(argument_to_parameter), Some(omitted_default_parameter_indices)) = (
        relation.get("argumentToParameter"),
        relation.get("omittedDefaultParameterIndices"),
    ) {
        let call_start = relation["start"].as_u64().ok_or_else(|| {
            invalid("Kotlin exact call has no source start for argument bindings")
        })?;
        let call_end = relation["end"]
            .as_u64()
            .ok_or_else(|| invalid("Kotlin exact call has no source end for argument bindings"))?;
        let descriptor = relation["targetJvmDescriptor"].as_str().ok_or_else(|| {
            invalid("Kotlin exact call has no target descriptor for argument bindings")
        })?;
        crate::semantic_validation::validate_kotlin_call_argument_bindings(
            call_start,
            call_end,
            descriptor,
            argument_to_parameter,
            omitted_default_parameter_indices,
        )?;
        fact["argumentBindings"] = json!({
            "schema":"codeclew-call-argument-bindings/1.0",
            "argumentToParameter":argument_to_parameter,
            "omittedDefaultParameterIndices":omitted_default_parameter_indices
        });
    }
    Ok(fact)
}

pub(super) fn project_facts(
    facts: Vec<(Value, String)>,
) -> Result<Vec<(Value, String)>, ClewError> {
    let compiler = facts.iter().find(|(fact, _)| fact["schema"].as_str().is_some_and(|s| s.starts_with("codeclew-kotlin-index-metadata/")))
        .map(|(fact, _)| json!({"projectCompilerVersion":fact["projectCompilerVersion"],"analyzerCompilerVersion":fact["analyzerCompilerVersion"],
            "projectSemantics":fact["kotlinProjectSemantics"],"semanticEngine":fact["kotlinSemanticEngine"]}));
    let mut flows = BTreeMap::new();
    for (fact, binding) in &facts {
        if let Some(declarations) = fact["declarations"].as_array() {
            for declaration in declarations {
                if let (Some(symbol), Some(flow)) = (
                    declaration["documentationSymbol"].as_str(),
                    declaration.get("documentation"),
                ) && flows
                    .insert(symbol.to_owned(), (flow.clone(), binding.clone()))
                    .is_some()
                {
                    return Err(invalid(
                        "Kotlin documentation has duplicate compiler declaration identities",
                    ));
                }
            }
        }
    }
    let mut function_descriptors =
        BTreeMap::<(String, String, String), Vec<FunctionDescriptor>>::new();
    for (fact, descriptor_binding) in &facts {
        if fact["schema"] != "declaration-descriptor/0.1" || fact["declarationKind"] != "FUNCTION" {
            continue;
        }
        if fact.get("attributeCoverage").is_some() || fact.get("sourceRowHash").is_some() {
            continue;
        }
        let checked = without_capture_scope(fact);
        crate::semantic_validation::validate_declaration_descriptor_fact(&checked)?;
        let (Some(file), Some(callable), Some(symbol), Some(start), Some(end)) = (
            checked["file"].as_str(),
            checked["compilerCallableId"].as_str(),
            checked["symbolIdentity"].as_str(),
            checked["start"].as_u64(),
            checked["end"].as_u64(),
        ) else {
            continue;
        };
        let descriptor_prefix = format!("callable:{callable}#jvm:");
        let Some(descriptor) = symbol.strip_prefix(&descriptor_prefix) else {
            continue;
        };
        if start >= end
            || checked["jvmDescriptor"]
                .as_str()
                .is_some_and(|declared| declared != descriptor)
        {
            continue;
        }
        crate::semantic_validation::validate_kotlin_full_symbol_identity(symbol)?;
        function_descriptors
            .entry((fact_scope(fact), file.to_owned(), callable.to_owned()))
            .or_default()
            .push(FunctionDescriptor {
                symbol: symbol.to_owned(),
                compiler_callable_id: callable.to_owned(),
                jvm_descriptor: descriptor.to_owned(),
                start,
                end,
                binding: descriptor_binding.clone(),
            });
    }

    // Preserve occurrence-level target uncertainty before flattening typed
    // relation boundaries to the service's stable boundary codes.
    let mut blocked_occurrences = BTreeSet::new();
    let mut target_resolution_boundaries = Vec::new();
    for (fact, _) in &facts {
        if fact["schema"] != "declaration-relation-boundary/0.1" {
            continue;
        }
        let checked = without_capture_scope(fact);
        crate::semantic_validation::validate_declaration_relation_boundary(&checked)?;
        if is_target_resolution_boundary(fact) {
            target_resolution_boundaries.push(fact.clone());
        }
    }

    let mut calls_by_occurrence = BTreeMap::<CallOccurrence, Vec<ProjectedCall>>::new();
    let mut call_boundaries = Vec::<(Value, String)>::new();
    let mut projected_calls = Vec::new();
    let mut local_cfg_candidates = BTreeMap::<
        (String, String, String),
        Vec<(crate::thread_flow_cfg::LocalCfgPayload, String)>,
    >::new();
    let mut local_cfg_boundaries = Vec::<(Value, String)>::new();
    let mut raw_local_cfg_boundaries = Vec::<(Value, String)>::new();
    for (fact, binding) in &facts {
        if fact["schema"] == crate::thread_flow_cfg::LOCAL_CFG_SCHEMA {
            let checked = without_capture_scope(fact);
            let graph: crate::thread_flow_cfg::LocalCfgPayload = serde_json::from_value(checked)
                .map_err(|_| {
                    invalid("Kotlin local CFG does not match the closed compiler graph contract")
                })?;
            crate::thread_flow_cfg::validate(&graph)?;
            let scope = fact_scope(fact);
            let key = (
                scope,
                graph.file.clone(),
                graph.owner_symbol_identity.clone(),
            );
            local_cfg_candidates
                .entry(key)
                .or_default()
                .push((graph, binding.clone()));
            continue;
        }
        if fact["schema"] == crate::thread_flow_cfg::LOCAL_CFG_BOUNDARY_SCHEMA {
            let checked = without_capture_scope(fact);
            crate::thread_flow_cfg::validate_boundary(&checked)?;
            raw_local_cfg_boundaries.push((fact.clone(), binding.clone()));
            continue;
        }
        if fact["schema"] != "declaration-relation/0.1" || fact["kind"] != "CALLS" {
            continue;
        }
        let checked = without_capture_scope(fact);
        crate::semantic_validation::validate_declaration_relation_fact(&checked)?;
        let Some(occurrence) = occurrence(fact) else {
            return Err(invalid(
                "Kotlin CALLS relation has no compiler source occurrence",
            ));
        };
        let partial =
            fact.get("attributeCoverage").is_some() || fact.get("sourceRowHash").is_some();
        let target_callable = fact["targetCompilerCallableId"].as_str();
        let target_descriptor = fact["targetJvmDescriptor"].as_str();
        let exact_target = match (target_callable, target_descriptor) {
            (Some(callable), Some(descriptor)) => {
                fact["target"] == format!("callable:{callable}#jvm:{descriptor}")
            }
            _ => false,
        };
        if partial || !exact_target {
            blocked_occurrences.insert(occurrence.clone());
            call_boundaries.push((
                json!({
                    "kind":"BOUNDARY",
                    "code":if partial { "KOTLIN_CALL_RELATION_PARTIAL" } else { "KOTLIN_CALL_TARGET_NOT_EXACT" }
                }),
                binding.clone(),
            ));
            continue;
        }
        if blocked_occurrences.contains(&occurrence)
            || target_resolution_boundaries
                .iter()
                .any(|boundary| boundary_applies_to_call(boundary, fact))
        {
            continue;
        }
        let owner_id = fact["owner"].as_str().unwrap_or_default();
        let (scope, file, start, end) = &occurrence;
        let owners = function_descriptors
            .get(&(scope.clone(), file.clone(), owner_id.to_owned()))
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .filter(|descriptor| descriptor.start <= *start && *end <= descriptor.end)
            .collect::<Vec<_>>();
        if owners.len() != 1 {
            blocked_occurrences.insert(occurrence.clone());
            call_boundaries.push((
                json!({
                    "kind":"BOUNDARY",
                    "code":"KOTLIN_CALL_OWNER_NOT_UNIQUE"
                }),
                binding.clone(),
            ));
            continue;
        }
        calls_by_occurrence
            .entry(occurrence.clone())
            .or_default()
            .push(ProjectedCall {
                occurrence,
                fact: exact_call_fact(fact, owners[0], binding)?,
                binding: binding.clone(),
            });
    }

    for (occurrence, mut calls) in calls_by_occurrence {
        if blocked_occurrences.contains(&occurrence) {
            continue;
        }
        let targets: BTreeSet<_> = calls
            .iter()
            .filter_map(|call| call.fact["targetIdentity"].as_str())
            .collect();
        let owners: BTreeSet<_> = calls
            .iter()
            .filter_map(|call| call.fact["sourceIdentity"].as_str())
            .collect();
        if targets.len() != 1 || owners.len() != 1 {
            blocked_occurrences.insert(occurrence);
            if let Some(call) = calls.first() {
                call_boundaries.push((
                    json!({
                        "kind":"BOUNDARY",
                        "code":"KOTLIN_CALL_RELATION_CONFLICT"
                    }),
                    call.binding.clone(),
                ));
            }
            continue;
        }
        // Identical compiler records for one occurrence share one normalized
        // relation. Keep one original CAS evidence binding deterministically.
        calls.sort_by(|left, right| left.binding.cmp(&right.binding));
        let call = calls.remove(0);
        debug_assert_eq!(call.occurrence, occurrence);
        // Appended after the source facts below to preserve their stable order.
        projected_calls.push((call.fact, call.binding));
    }

    let mut projected_local_cfg = Vec::<(Value, String)>::new();
    for (fact, binding) in &raw_local_cfg_boundaries {
        let provider = match fact["provider"].as_str() {
            Some("K2_FIR_CFG") => "K2_FIR_CFG",
            Some("CODECLEW_LOCAL_CFG_NORMALIZER") => "CODECLEW_LOCAL_CFG_NORMALIZER",
            _ => return Err(invalid("Kotlin local CFG boundary has unknown provider")),
        };
        local_cfg_boundaries.push(local_cfg_boundary(
            fact_scope(fact),
            fact["ownerSymbolIdentity"].as_str().map(str::to_owned),
            fact["file"].as_str().map(str::to_owned),
            fact["compilerGraphName"].as_str().map(str::to_owned),
            fact["code"]
                .as_str()
                .unwrap_or("LOCAL_CFG_NORMALIZATION_FAILED"),
            binding.clone(),
            provider,
            fact["rawRowHash"].as_str().map(str::to_owned),
        )?);
    }
    for ((scope, file, owner), mut candidates) in local_cfg_candidates {
        let unknown_boundary_applies = raw_local_cfg_boundaries.iter().any(|(boundary, _)| {
            let boundary_scope = fact_scope(boundary);
            let scope_matches = boundary_scope == scope;
            let owner_matches = boundary["ownerSymbolIdentity"]
                .as_str()
                .is_none_or(|value| value == owner);
            let file_matches = boundary["file"].as_str().is_none_or(|value| value == file);
            scope_matches && owner_matches && file_matches
        });
        if unknown_boundary_applies {
            continue;
        }
        let scoped_owner_descriptors = function_descriptors
            .iter()
            .filter(|((candidate_scope, _, _), _)| candidate_scope == &scope)
            .flat_map(|((_, candidate_file, _), descriptors)| {
                descriptors
                    .iter()
                    .filter(|descriptor| descriptor.symbol == owner)
                    .map(move |descriptor| (candidate_file, descriptor))
            })
            .collect::<Vec<_>>();
        let descriptors = exact_function_descriptors(&function_descriptors, &scope, &file, &owner);
        if descriptors.len() != 1
            || scoped_owner_descriptors.len() != 1
            || scoped_owner_descriptors[0].0 != &file
            || candidates.len() != 1
        {
            let binding = candidates
                .first()
                .map(|(_, binding)| binding.clone())
                .unwrap_or_default();
            let graph_name = candidates
                .first()
                .map(|(graph, _)| graph.compiler_graph_name.clone());
            local_cfg_boundaries.push(local_cfg_boundary(
                scope,
                Some(owner),
                Some(file),
                graph_name,
                "KOTLIN_LOCAL_CFG_OWNER_NOT_UNIQUE",
                binding,
                "CODECLEW_LOCAL_CFG_NORMALIZER",
                None,
            )?);
            continue;
        }
        let descriptor = descriptors[0];
        let (graph, graph_binding) = candidates.pop().expect("one CFG candidate");
        if graph.owner_symbol_identity != descriptor.symbol || graph.file != file {
            local_cfg_boundaries.push(local_cfg_boundary(
                scope,
                Some(owner),
                Some(file),
                Some(graph.compiler_graph_name),
                "KOTLIN_LOCAL_CFG_OWNER_MISMATCH",
                graph_binding,
                "CODECLEW_LOCAL_CFG_NORMALIZER",
                None,
            )?);
            continue;
        }
        let fact = serde_json::to_value(ProjectedLocalCfg {
            schema: DOCUMENTATION_LOCAL_CFG_SCHEMA,
            kind: "LOCAL_CFG",
            graph,
            owner_symbol_identity: descriptor.symbol.clone(),
            file: file.clone(),
            scope,
            owner_start: descriptor.start,
            owner_end: descriptor.end,
            graph_evidence_binding: graph_binding.clone(),
            descriptor_evidence_binding: descriptor.binding.clone(),
        })
        .map_err(|_| invalid("Kotlin local CFG projection cannot be serialized"))?;
        projected_local_cfg.push((fact, graph_binding));
    }

    let mut output = Vec::new();
    for (mut fact, binding) in facts {
        let schema = fact["schema"].as_str().unwrap_or("");
        if schema.starts_with("codeclew-kotlin-index-metadata/") {
            if let Some(boundaries) = fact["buildModelBoundaries"].as_array() {
                for boundary in boundaries {
                    output.push((json!({"kind":"BOUNDARY","code": boundary.as_str()
                        .or_else(|| boundary["code"].as_str()).unwrap_or("KOTLIN_BUILD_MODEL_BOUNDARY")}), binding.clone()));
                }
            }
        } else if schema == "declaration-descriptor/0.1" {
            let Some(symbol) = fact["symbolIdentity"].as_str().map(str::to_owned) else {
                continue;
            };
            fact["kind"] = json!("DECLARATION");
            if let Some(compiler) = &compiler {
                fact["analysis"] = compiler.clone();
            }
            let callable = fact["compilerCallableId"].as_str().unwrap_or("");
            fact["name"] = json!(callable.rsplit(['.', '/']).next().unwrap_or(""));
            let combined_binding = if let Some((flow, flow_binding)) = flows.remove(&symbol) {
                fact["documentation"] = flow;
                digest(&json!([binding, flow_binding]))?
            } else {
                binding
            };
            output.push((fact, combined_binding));
        } else if schema.contains("boundary/") {
            output.push((json!({"kind":"BOUNDARY", "code":fact["code"].as_str().unwrap_or("KOTLIN_ANALYSIS_BOUNDARY")}), binding));
        }
    }
    output.extend(projected_calls);
    output.extend(call_boundaries);
    output.extend(projected_local_cfg);
    output.extend(local_cfg_boundaries);
    if !output.iter().any(|(fact, _)| fact["kind"] == "DECLARATION") {
        return Err(ClewError::new(
            crate::error::ErrorCode::IncompleteSemanticAnalysis,
            "Kotlin compiler declarations are unavailable; resolve compilation dependencies before documenting behavior",
        ));
    }
    Ok(output)
}

fn exact_function_descriptors<'a>(
    descriptors: &'a BTreeMap<(String, String, String), Vec<FunctionDescriptor>>,
    scope: &str,
    file: &str,
    symbol: &str,
) -> Vec<&'a FunctionDescriptor> {
    let Some(identity) = symbol.strip_prefix("callable:") else {
        return Vec::new();
    };
    let Some((callable, descriptor)) = identity.split_once("#jvm:") else {
        return Vec::new();
    };
    descriptors
        .get(&(scope.to_owned(), file.to_owned(), callable.to_owned()))
        .into_iter()
        .flatten()
        .filter(|candidate| candidate.symbol == symbol && candidate.jvm_descriptor == descriptor)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::documentation::{analysis, check, cli, model::*, render, store};

    fn function_descriptor(
        callable: &str,
        jvm: &str,
        file: &str,
        start: u64,
        end: u64,
        scope: &str,
    ) -> (Value, String) {
        let owner = callable
            .rsplit_once('.')
            .or_else(|| callable.rsplit_once('/'))
            .unwrap()
            .0;
        let symbol = format!("callable:{callable}#jvm:{jvm}");
        (
            json!({
                "schema":"declaration-descriptor/0.1",
                "file":file,"start":start,"end":end,
                "symbolIdentity":symbol,"declarationKind":"FUNCTION",
                "ownerIdentity":format!("class:{owner}"),
                "containment":[format!("class:{owner}")],
                "visibility":"public","effectiveVisibility":"public",
                "exportBoundary":"PUBLIC_API","modality":"FINAL",
                "compilerCallableId":callable,"jvmDescriptor":jvm,
                "isOverride":false,"returnType":"kotlin/Unit","returnNullable":false,
                "parameterTypes":[],"typeParameters":[],
                "module":":","sourceSet":"main",
                "sourceProvenance":"COMPILER_UTF16_RANGE_TO_UTF8_BYTES",
                "compilerAuthority":"fir-facts-extractor/0.6",
                "resolution":"PROVEN","provider":"K2_FIR",
                "scope":{"compilation":scope}
            }),
            format!("descriptor-binding:{callable}:{jvm}:{scope}"),
        )
    }

    fn call_relation(
        owner: &str,
        target_callable: &str,
        target_jvm: &str,
        file: &str,
        start: u64,
        end: u64,
        scope: &str,
    ) -> (Value, String) {
        (
            json!({
                "schema":"declaration-relation/0.1",
                "file":file,"start":start,"end":end,
                "kind":"CALLS","owner":owner,
                "target":format!("callable:{target_callable}#jvm:{target_jvm}"),
                "targetCompilerCallableId":target_callable,
                "targetJvmDescriptor":target_jvm,
                "resolution":"PROVEN","provider":"K2_FIR",
                "cfgNodeIds":[],
                "sourceProvenance":"COMPILER_UTF16_RANGE_TO_UTF8_BYTES",
                "orderProvenance":"FIR_SOURCE_RANGE",
                "receiverSelection":"EXPLICIT","receiverType":"p/Api",
                "argumentToParameter":[],"omittedDefaultParameterIndices":[],
                "scope":{"compilation":scope}
            }),
            format!("original-call-binding:{file}:{start}:{end}:{scope}"),
        )
    }

    fn local_cfg_fact(
        owner: &str,
        graph_name: &str,
        file: &str,
        scope: &str,
    ) -> (Value, String, crate::thread_flow_cfg::LocalCfgPayload) {
        use crate::thread_flow_cfg::{
            LOCAL_CFG_SCHEMA, LocalCfgEdge, LocalCfgEdgeKind, LocalCfgNode, LocalCfgNodeRole,
            LocalCfgPayload, LocalCfgSourceRange,
        };
        let mut graph = LocalCfgPayload {
            schema: LOCAL_CFG_SCHEMA.into(),
            graph_id: String::new(),
            owner_symbol_identity: owner.into(),
            file: file.into(),
            compiler_graph_name: graph_name.into(),
            provider: "K2_FIR_CFG".into(),
            source_provenance: "COMPILER_UTF16_RANGE_TO_UTF8_BYTES".into(),
            nodes: vec![
                LocalCfgNode {
                    node_id: 0,
                    role: LocalCfgNodeRole::Entry,
                    source: Some(LocalCfgSourceRange { start: 12, end: 80 }),
                },
                LocalCfgNode {
                    node_id: 1,
                    role: LocalCfgNodeRole::Return,
                    source: Some(LocalCfgSourceRange { start: 60, end: 66 }),
                },
            ],
            edges: vec![LocalCfgEdge {
                source_node_id: 0,
                target_node_id: 1,
                kind: LocalCfgEdgeKind::Return,
                label: Some("CompilerReturn".into()),
            }],
        };
        graph.graph_id = crate::canonical::hash(&graph).unwrap();
        let mut fact = serde_json::to_value(&graph).unwrap();
        fact["scope"] = json!({"compilation":scope});
        (fact, format!("graph-binding:{owner}:{scope}"), graph)
    }

    fn local_cfg_boundary_fact(
        owner: Option<&str>,
        file: Option<&str>,
        scope: &str,
        provider: &str,
        code: &str,
    ) -> (Value, String) {
        (
            json!({
                "schema":"local-cfg-boundary/0.1",
                "ownerSymbolIdentity":owner,
                "file":file,
                "compilerGraphName":"opaque compiler display name",
                "stage":"NORMALIZE",
                "code":code,
                "resolution":"UNKNOWN",
                "provider":provider,
                "sourceProvenance":"COMPILER_UTF16_RANGE_TO_UTF8_BYTES",
                "rawRowHash":format!("sha256:{}", "a".repeat(64)),
                "scope":{"compilation":scope}
            }),
            format!("sha256:{}", "b".repeat(64)),
        )
    }

    #[test]
    fn local_cfg_projects_only_with_exact_scoped_function_binding() {
        let (descriptor, descriptor_binding) = function_descriptor(
            "p/Answer.next",
            "(I)I",
            "src/main/kotlin/Answer.kt",
            10,
            90,
            "scope-a",
        );
        let symbol = descriptor["symbolIdentity"].as_str().unwrap();
        let (graph, graph_binding, original) =
            local_cfg_fact(symbol, "next", "src/main/kotlin/Answer.kt", "scope-a");
        let projected = project_facts(vec![
            (descriptor, descriptor_binding.clone()),
            (graph, graph_binding.clone()),
        ])
        .unwrap();
        let (local_cfg, binding) = projected
            .iter()
            .find(|(fact, _)| fact["schema"] == DOCUMENTATION_LOCAL_CFG_SCHEMA)
            .expect("exact function CFG is retained");
        assert_eq!(binding, &graph_binding);
        assert_eq!(local_cfg["kind"], "LOCAL_CFG");
        assert_eq!(local_cfg["scope"], "scope-a");
        assert_eq!(local_cfg["ownerStart"], 10);
        assert_eq!(local_cfg["ownerEnd"], 90);
        assert_eq!(local_cfg["graphEvidenceBinding"], graph_binding);
        assert_eq!(local_cfg["descriptorEvidenceBinding"], descriptor_binding);
        assert_eq!(
            serde_json::from_value::<crate::thread_flow_cfg::LocalCfgPayload>(
                local_cfg["graph"].clone()
            )
            .unwrap(),
            original,
            "the compiler graph remains unchanged"
        );
    }

    #[test]
    fn local_cfg_with_wrong_scope_or_duplicate_owner_fails_closed() {
        let (descriptor, _) = function_descriptor(
            "p/Answer.next",
            "(I)I",
            "src/main/kotlin/Answer.kt",
            10,
            90,
            "scope-a",
        );
        let symbol = descriptor["symbolIdentity"].as_str().unwrap();
        let (wrong_scope, binding, _) =
            local_cfg_fact(symbol, "next", "src/main/kotlin/Answer.kt", "scope-b");
        let wrong_scope = project_facts(vec![
            (descriptor.clone(), "descriptor-binding".into()),
            (wrong_scope, binding),
        ])
        .unwrap();
        assert!(
            !wrong_scope
                .iter()
                .any(|(fact, _)| fact["schema"] == DOCUMENTATION_LOCAL_CFG_SCHEMA)
        );
        assert!(wrong_scope.iter().any(|(fact, _)| {
            fact["kind"] == "LOCAL_CFG_BOUNDARY"
                && fact["code"] == "KOTLIN_LOCAL_CFG_OWNER_NOT_UNIQUE"
        }));

        let (graph, binding, _) =
            local_cfg_fact(symbol, "next", "src/main/kotlin/Answer.kt", "scope-a");
        let duplicated = project_facts(vec![
            (descriptor.clone(), "descriptor-binding".into()),
            (graph.clone(), binding.clone()),
            (graph, binding),
        ])
        .unwrap();
        assert!(
            !duplicated
                .iter()
                .any(|(fact, _)| fact["schema"] == DOCUMENTATION_LOCAL_CFG_SCHEMA)
        );
        assert!(duplicated.iter().any(|(fact, _)| {
            fact["kind"] == "LOCAL_CFG_BOUNDARY"
                && fact["code"] == "KOTLIN_LOCAL_CFG_OWNER_NOT_UNIQUE"
        }));

        let file_a = "src/main/kotlin/p/Answer.kt";
        let file_b = "src/test/kotlin/p/Answer.kt";
        let (other_scope_exact, _) =
            function_descriptor("p/Answer.next", "(I)I", file_b, 10, 90, "scope-b");
        let (same_bucket_other_overload, _) = function_descriptor(
            "p/Answer.next",
            "(Ljava/lang/String;)I",
            file_a,
            10,
            90,
            "scope-a",
        );
        let (graph, binding, _) =
            local_cfg_fact(symbol, "opaque compiler label", file_a, "scope-a");
        let cross_scope_borrow = project_facts(vec![
            (other_scope_exact, "other-scope-descriptor".into()),
            (same_bucket_other_overload, "same-bucket-overload".into()),
            (graph, binding),
        ])
        .unwrap();
        assert!(
            !cross_scope_borrow
                .iter()
                .any(|(fact, _)| fact["schema"] == DOCUMENTATION_LOCAL_CFG_SCHEMA)
        );

        let (exact_a, _) = function_descriptor("p/Answer.next", "(I)I", file_a, 10, 90, "scope-a");
        let (same_owner_other_file, _) =
            function_descriptor("p/Answer.next", "(I)I", file_b, 10, 90, "scope-a");
        let (graph, binding, _) = local_cfg_fact(symbol, "opaque", file_a, "scope-a");
        let cross_file_duplicate = project_facts(vec![
            (exact_a, "exact-file-descriptor".into()),
            (same_owner_other_file, "other-file-descriptor".into()),
            (graph, binding),
        ])
        .unwrap();
        assert!(
            !cross_file_duplicate
                .iter()
                .any(|(fact, _)| fact["schema"] == DOCUMENTATION_LOCAL_CFG_SCHEMA)
        );
    }

    #[test]
    fn matching_local_cfg_boundary_vetoes_graph_and_keeps_provider() {
        let file = "src/main/kotlin/p/Answer.kt";
        let (descriptor, _) = function_descriptor("p/Answer.next", "(I)I", file, 10, 90, "scope-a");
        let owner = descriptor["symbolIdentity"].as_str().unwrap();
        let (graph, graph_binding, _) = local_cfg_fact(owner, "opaque", file, "scope-a");
        let (boundary, _) = local_cfg_boundary_fact(
            Some(owner),
            Some(file),
            "scope-a",
            "CODECLEW_LOCAL_CFG_NORMALIZER",
            "INVALID_LOCAL_CFG",
        );
        let projected = project_facts(vec![
            (descriptor.clone(), "descriptor-binding".into()),
            (graph, graph_binding),
            (boundary, "boundary-binding".into()),
        ])
        .unwrap();
        assert!(
            !projected
                .iter()
                .any(|(fact, _)| fact["schema"] == DOCUMENTATION_LOCAL_CFG_SCHEMA)
        );
        let boundary = projected
            .iter()
            .find(|(fact, _)| fact["schema"] == LOCAL_CFG_BOUNDARY_EVIDENCE_SCHEMA)
            .expect("matching UNKNOWN boundary is retained");
        assert_eq!(boundary.0["kind"], "LOCAL_CFG_BOUNDARY");
        assert_eq!(boundary.0["provider"], "CODECLEW_LOCAL_CFG_NORMALIZER");
        assert_eq!(boundary.0["ownerSymbolIdentity"], owner);
        assert_eq!(boundary.0["scope"], "scope-a");

        let (graph, graph_binding, _) = local_cfg_fact(owner, "opaque", file, "scope-a");
        let (unrelated, _) = local_cfg_boundary_fact(
            Some("callable:p/Other.next#jvm:(I)I"),
            Some(file),
            "scope-b",
            "CODECLEW_LOCAL_CFG_NORMALIZER",
            "INVALID_LOCAL_CFG",
        );
        let unrelated = project_facts(vec![
            (descriptor, "descriptor-binding".into()),
            (graph, graph_binding),
            (unrelated, "unrelated-boundary-binding".into()),
        ])
        .unwrap();
        assert!(
            unrelated
                .iter()
                .any(|(fact, _)| fact["schema"] == DOCUMENTATION_LOCAL_CFG_SCHEMA)
        );
    }

    #[test]
    fn local_cfg_flows_from_compiler_fact_to_compact_exact_symbol_context() {
        use crate::thread_flow_cfg::{
            LOCAL_CFG_SCHEMA, LocalCfgEdge, LocalCfgEdgeKind, LocalCfgNode, LocalCfgNodeRole,
            LocalCfgPayload, LocalCfgSourceRange,
        };

        let file = "src/main/kotlin/p/Answer.kt";
        let scope = "scope-a";
        let source = "package p\r\n// π prefix\r\nclass Answer {\r\n  fun next(value: Int): Int {\r\n    return value + 1\r\n  }\r\n}\r\n";
        let owner_start = source.find("  fun next").unwrap();
        let owner_end = owner_start + source[owner_start..].find("\r\n  }\r\n}").unwrap() + 5;
        let return_start = source.find("    return value + 1").unwrap();
        let return_end = return_start + "    return value + 1".len();
        let (mut descriptor, _) = function_descriptor(
            "p/Answer.next",
            "(I)I",
            file,
            owner_start as u64,
            owner_end as u64,
            scope,
        );
        descriptor["startLine"] = json!(4);
        descriptor["endLine"] = json!(6);
        descriptor["lineProvenance"] = json!("UTF8_BYTE_RANGE_OVER_COMPILATION_SOURCE");
        let descriptor_binding = format!("sha256:{}", "1".repeat(64));
        let mut graph = LocalCfgPayload {
            schema: LOCAL_CFG_SCHEMA.into(),
            graph_id: String::new(),
            owner_symbol_identity: descriptor["symbolIdentity"].as_str().unwrap().into(),
            file: file.into(),
            compiler_graph_name: "opaque compiler display name".into(),
            provider: "K2_FIR_CFG".into(),
            source_provenance: "COMPILER_UTF16_RANGE_TO_UTF8_BYTES".into(),
            nodes: vec![
                LocalCfgNode {
                    node_id: 0,
                    role: LocalCfgNodeRole::Entry,
                    source: Some(LocalCfgSourceRange {
                        start: owner_start as u64,
                        end: owner_end as u64,
                    }),
                },
                LocalCfgNode {
                    node_id: 1,
                    role: LocalCfgNodeRole::Return,
                    source: Some(LocalCfgSourceRange {
                        start: return_start as u64,
                        end: return_end as u64,
                    }),
                },
            ],
            edges: vec![LocalCfgEdge {
                source_node_id: 0,
                target_node_id: 1,
                kind: LocalCfgEdgeKind::Return,
                label: Some("CompilerReturn".into()),
            }],
        };
        graph.graph_id = crate::canonical::hash(&graph).unwrap();
        let graph_binding = format!("sha256:{}", "2".repeat(64));
        let mut graph_fact = serde_json::to_value(&graph).unwrap();
        graph_fact["scope"] = json!({"compilation":scope});

        let projected = project_facts(vec![
            (descriptor, descriptor_binding.clone()),
            (graph_fact, graph_binding.clone()),
        ])
        .unwrap();
        let mut service: Service = serde_json::from_value(json!({
            "schema":"codeclew-documentation-service/1.0",
            "id":"svc","title":"Answer service",
            "repositoryId":"svc","repository":"https://example.invalid/svc",
            "language":"kotlin","profile":"kotlin-jvm-maven-analysis",
            "compilations":[scope],"targetRef":"main"
        }))
        .unwrap();
        service.source_link_template = Some("{repository}/blob/{revision}/{file}".into());
        let files = BTreeMap::from([(file.into(), source.into())]);
        let revision = "a".repeat(40);
        let evidence = analysis::project(
            &service,
            &revision,
            &digest(&service).unwrap(),
            "DEVELOPMENT",
            "PARTIAL",
            projected.clone(),
            &files,
            false,
        )
        .unwrap();
        let owner = graph.owner_symbol_identity.clone();
        let cfg = evidence
            .observations
            .values()
            .find(|observation| observation.kind == "LOCAL_CFG")
            .expect("validated local CFG is retained");
        assert_eq!(cfg.symbol, owner);
        assert_eq!(
            cfg.normalized["graph"],
            serde_json::to_value(&graph).unwrap()
        );
        assert_eq!(cfg.normalized["graphEvidenceBinding"], graph_binding);
        assert_eq!(
            cfg.normalized["descriptorEvidenceBinding"],
            descriptor_binding
        );
        assert_eq!(cfg.normalized["nodeCitations"][0]["byteStart"], 0);
        let source_record = &evidence.sources[&cfg.source_ids[0]];
        assert_eq!(source_record.text, &source[owner_start..owner_end]);
        assert_eq!(source_record.authority, "EXACT_SNAPSHOT_TEXT");
        assert!(source_record.occurrence.is_none());
        assert!(
            !crate::documentation::local_cfg::LocalCfgEvidence::source_bound(
                &cfg.normalized,
                source_record,
                "other-service",
                &revision
            )
        );
        assert!(
            !crate::documentation::local_cfg::LocalCfgEvidence::source_bound(
                &cfg.normalized,
                source_record,
                &service.id,
                &"c".repeat(40)
            )
        );
        let mut wrong_authority = source_record.clone();
        wrong_authority.authority = "UNPROVEN_SOURCE".into();
        assert!(
            !crate::documentation::local_cfg::LocalCfgEvidence::source_bound(
                &cfg.normalized,
                &wrong_authority,
                &service.id,
                &revision
            )
        );
        let mut wrong_site_authority = cfg.normalized.clone();
        wrong_site_authority["sourceSite"]["authority"] = json!("UNPROVEN_SOURCE");
        assert!(
            !crate::documentation::local_cfg::LocalCfgEvidence::source_bound(
                &wrong_site_authority,
                &wrong_authority,
                &service.id,
                &revision
            )
        );

        let transformed = analysis::project(
            &service,
            &revision,
            &digest(&service).unwrap(),
            "DEVELOPMENT",
            "PARTIAL",
            projected.clone(),
            &files,
            true,
        )
        .unwrap();
        let transformed_cfg = transformed
            .observations
            .values()
            .find(|observation| observation.kind == "LOCAL_CFG")
            .unwrap();
        let transformed_source = &transformed.sources[&transformed_cfg.source_ids[0]];
        assert_eq!(
            transformed_source.authority,
            crate::generation_service::TRANSFORMED_SOURCE_AUTHORITY
        );
        assert!(transformed_source.url.is_none());
        assert!(
            crate::documentation::local_cfg::LocalCfgEvidence::source_bound(
                &transformed_cfg.normalized,
                transformed_source,
                &service.id,
                &revision
            )
        );

        let source_missing = analysis::project(
            &service,
            &revision,
            &digest(&service).unwrap(),
            "DEVELOPMENT",
            "PARTIAL",
            projected,
            &BTreeMap::new(),
            false,
        )
        .unwrap();
        let source_boundary = source_missing
            .observations
            .values()
            .find(|observation| observation.kind == "LOCAL_CFG_BOUNDARY")
            .expect("missing retained source is an owner-scoped boundary");
        assert!(
            source_missing
                .observations
                .values()
                .all(|observation| observation.kind != "LOCAL_CFG")
        );
        assert_eq!(
            source_boundary.normalized["provider"],
            "CODECLEW_LOCAL_CFG_NORMALIZER"
        );
        assert_eq!(source_boundary.normalized["evidenceBinding"], graph_binding);

        let checked = check::Check {
            schema: "codeclew-documentation-check/1.0".into(),
            input_digest: "synthetic-input".into(),
            context_digest: "synthetic-context".into(),
            services: BTreeMap::from([(service.id.clone(), evidence.clone())]),
            dependencies: evidence.observations.clone(),
            unresolved: BTreeMap::new(),
            interactions: BTreeMap::new(),
            scenarios: BTreeMap::new(),
            source_inputs: None,
            composition: None,
        };
        let args = cli::ContextArgs {
            root: std::path::PathBuf::new(),
            service: Some(service.id.clone()),
            scenario: None,
            entrypoint: None,
            symbols: vec![owner],
            source_ids: Vec::new(),
            dependency_ids: Vec::new(),
            format: cli::ContextFormat::Compact,
            refresh: false,
            snapshot: None,
            cursor: None,
            limit: 20,
        };
        let context = cli::context_items(&checked, &args, None).unwrap();
        let dependency = context
            .iter()
            .find(|item| item["kind"] == "DEPENDENCY" && item["record"]["kind"] == "LOCAL_CFG")
            .expect("the exact symbol selects its owned local CFG");
        assert!(dependency["record"]["normalized"].get("graph").is_none());
        assert_eq!(
            dependency["record"]["normalized"]["graphSummary"]["graphId"],
            graph.graph_id
        );
        assert_eq!(
            dependency["record"]["normalized"]["graphSummary"]["provider"],
            "K2_FIR_CFG"
        );
        assert_eq!(
            dependency["record"]["normalized"]["graphSummary"]["nodeCount"],
            2
        );
        assert_eq!(
            dependency["record"]["normalized"]["graphSummary"]["edgeCount"],
            1
        );
        assert_eq!(
            dependency["record"]["normalized"]["graphSummary"]["nodeCitationCount"],
            2
        );
        assert!(
            dependency["record"]["normalized"]
                .get("nodeCitations")
                .is_none()
        );
        assert!(context.iter().any(|item| {
            item["kind"] == "SOURCE" && item["record"]["text"] == &source[owner_start..owner_end]
        }));
        let source_missing_check = check::Check {
            schema: "codeclew-documentation-check/1.0".into(),
            input_digest: "synthetic-input".into(),
            context_digest: "synthetic-context".into(),
            services: BTreeMap::from([(service.id.clone(), source_missing.clone())]),
            dependencies: source_missing.observations.clone(),
            unresolved: BTreeMap::new(),
            interactions: BTreeMap::new(),
            scenarios: BTreeMap::new(),
            source_inputs: None,
            composition: None,
        };
        let missing_context = cli::context_items(&source_missing_check, &args, None).unwrap();
        assert!(missing_context.iter().any(|item| {
            item["kind"] == "DEPENDENCY"
                && item["record"]["kind"] == "LOCAL_CFG_BOUNDARY"
                && item["record"]["normalized"]["ownerSymbolIdentity"]
                    == graph.owner_symbol_identity
        }));
    }

    fn relation_boundary(
        stage: &str,
        code: &str,
        owner: &str,
        file: &str,
        start: u64,
        end: u64,
        scope: &str,
    ) -> (Value, String) {
        (
            json!({
                "schema":"declaration-relation-boundary/0.1",
                "file":file,"start":start,"end":end,
                "owner":owner,"stage":stage,"code":code,
                "resolution":"UNKNOWN","provider":if stage == "ORDER_PROVENANCE" { "K2_FIR_CFG" } else { "K2_FIR" },
                "scope":{"compilation":scope}
            }),
            format!("boundary-binding:{file}:{start}:{end}:{code}"),
        )
    }

    fn wide_relation_boundary(
        stage: &str,
        owner: Option<&str>,
        target: Option<&str>,
        scope: &str,
    ) -> (Value, String) {
        let mut fact = json!({
            "schema":"declaration-relation-boundary/0.1",
            "stage":stage,"code":"UNRESOLVED_RELATION_TARGET",
            "resolution":"UNKNOWN","provider":"K2_FIR",
            "relationKind":"CALLS","scope":{"compilation":scope}
        });
        if let Some(owner) = owner {
            fact["owner"] = json!(owner);
        }
        if let Some(target) = target {
            fact["target"] = json!(target);
        }
        (
            fact,
            format!("wide-boundary:{stage}:{owner:?}:{target:?}:{scope}"),
        )
    }

    fn projected_calls(facts: Vec<(Value, String)>) -> Vec<(Value, String)> {
        project_facts(facts).unwrap()
    }

    #[test]
    fn exact_calls_bind_to_the_unique_containing_function_and_preserve_original_binding() {
        let owner = "p/caller";
        let file = "src/main/kotlin/p/Calls.kt";
        let scope = ":/main";
        let (_, unrelated_binding) = function_descriptor(owner, "()V", file, 0, 10, scope);
        let (containing, _) = function_descriptor(owner, "(I)V", file, 20, 90, scope);
        let (call, binding) = call_relation(owner, "p/Api.pick", "()V", file, 40, 54, scope);
        let output = projected_calls(vec![
            (containing.clone(), "owner-second-overload".into()),
            (
                json!({"schema":"declaration-descriptor/0.1","declarationKind":"CLASS"}),
                unrelated_binding,
            ),
            (
                function_descriptor(owner, "()V", file, 0, 10, scope).0,
                "owner-first-overload".into(),
            ),
            (call, binding.clone()),
        ]);
        let (adapted, adapted_binding) = output
            .iter()
            .find(|(fact, _)| fact["schema"] == DOCUMENTATION_CALL_SCHEMA)
            .unwrap();
        assert_eq!(adapted["sourceIdentity"], containing["symbolIdentity"]);
        assert_eq!(adapted["targetIdentity"], "callable:p/Api.pick#jvm:()V");
        assert_eq!(adapted["relationKind"], "CALLS");
        assert_eq!(adapted["resolution"], "COMPILER_EXACT");
        assert_eq!(adapted["compilerResolution"], "PROVEN");
        assert_eq!(adapted["provider"], "K2_FIR");
        assert_eq!(adapted["targetCompilerCallableId"], "p/Api.pick");
        assert_eq!(adapted["targetJvmDescriptor"], "()V");
        assert_eq!(adapted["byteStart"], 40);
        assert_eq!(adapted_binding, &binding);
        assert!(!output.iter().any(|(fact, _)| fact["kind"] == "FLOW"));
    }

    #[test]
    fn exact_call_argument_binding_envelope_preserves_source_order_and_optional_names() {
        let owner = "p/caller";
        let file = "src/main/kotlin/p/Calls.kt";
        let scope = ":/main";
        let (containing, _) = function_descriptor(owner, "()V", file, 0, 100, scope);
        let descriptor = "(Ljava/lang/String;ILjava/lang/String;)Ljava/lang/String;";
        let (mut call, binding) =
            call_relation(owner, "p/Api.pick", descriptor, file, 40, 90, scope);
        call["argumentToParameter"] = json!([
            {
                "argumentStart":50,"argumentEnd":53,
                "argumentType":"kotlin/String","parameter":"last",
                "parameterIndex":2,"parameterType":"kotlin/String"
            },
            {
                "argumentStart":70,"argumentEnd":73,
                "argumentType":"kotlin/String","parameter":"first",
                "parameterIndex":0,"parameterType":"kotlin/String"
            }
        ]);
        call["omittedDefaultParameterIndices"] = json!([1]);
        let output = projected_calls(vec![
            (containing.clone(), "owner-binding".into()),
            (call, binding),
        ]);
        let (adapted, _) = output
            .iter()
            .find(|(fact, _)| fact["schema"] == DOCUMENTATION_CALL_SCHEMA)
            .unwrap();
        assert_eq!(
            adapted["argumentBindings"]["schema"],
            "codeclew-call-argument-bindings/1.0"
        );
        assert_eq!(
            adapted["argumentBindings"]["argumentToParameter"][0]["parameterIndex"],
            2
        );
        assert_eq!(
            adapted["argumentBindings"]["argumentToParameter"][1]["parameterIndex"],
            0
        );
        assert_eq!(
            adapted["argumentBindings"]["omittedDefaultParameterIndices"],
            json!([1])
        );
        assert!(
            adapted["argumentBindings"]["argumentToParameter"][0]
                .get("argumentName")
                .is_none()
        );

        let (mut without_mapping, binding) =
            call_relation(owner, "p/Api.pick", "()V", file, 20, 30, scope);
        without_mapping
            .as_object_mut()
            .unwrap()
            .remove("argumentToParameter");
        let output = projected_calls(vec![
            (containing.clone(), "owner-binding".into()),
            (without_mapping, binding),
        ]);
        let (adapted, _) = output
            .iter()
            .find(|(fact, _)| fact["schema"] == DOCUMENTATION_CALL_SCHEMA)
            .unwrap();
        assert!(adapted.get("argumentBindings").is_none());

        let (zero_argument_call, binding) =
            call_relation(owner, "p/Api.zero", "()V", file, 20, 21, scope);
        let output = projected_calls(vec![
            (containing.clone(), "owner-binding".into()),
            (zero_argument_call, binding),
        ]);
        let (adapted, _) = output
            .iter()
            .find(|(fact, _)| fact["schema"] == DOCUMENTATION_CALL_SCHEMA)
            .unwrap();
        assert_eq!(
            adapted["argumentBindings"]["argumentToParameter"],
            json!([])
        );
        assert_eq!(
            adapted["argumentBindings"]["omittedDefaultParameterIndices"],
            json!([])
        );

        let (mut all_default_call, binding) = call_relation(
            owner,
            "p/Api.defaulted",
            "(Ljava/lang/String;)V",
            file,
            30,
            31,
            scope,
        );
        all_default_call["omittedDefaultParameterIndices"] = json!([0]);
        let output = projected_calls(vec![
            (containing, "owner-binding".into()),
            (all_default_call, binding),
        ]);
        let (adapted, _) = output
            .iter()
            .find(|(fact, _)| fact["schema"] == DOCUMENTATION_CALL_SCHEMA)
            .unwrap();
        assert_eq!(
            adapted["argumentBindings"]["argumentToParameter"],
            json!([])
        );
        assert_eq!(
            adapted["argumentBindings"]["omittedDefaultParameterIndices"],
            json!([0])
        );
    }

    #[test]
    fn calls_reject_family_targets_ambiguous_owners_wrong_scope_and_partial_rows() {
        let owner = "p/caller";
        let file = "src/main/kotlin/p/Calls.kt";
        let scope = ":/main";
        let descriptor = function_descriptor(owner, "()V", file, 0, 100, scope);

        let mut family = call_relation(owner, "p/Api.pick", "()V", file, 20, 30, scope);
        family.0["target"] = json!("p/Api.pick");
        family
            .0
            .as_object_mut()
            .unwrap()
            .remove("targetCompilerCallableId");
        family
            .0
            .as_object_mut()
            .unwrap()
            .remove("targetJvmDescriptor");
        let output = projected_calls(vec![descriptor.clone(), family]);
        assert!(
            !output
                .iter()
                .any(|(fact, _)| fact["schema"] == DOCUMENTATION_CALL_SCHEMA)
        );
        assert!(
            output
                .iter()
                .any(|(fact, _)| fact["code"] == "KOTLIN_CALL_TARGET_NOT_EXACT")
        );

        let call = call_relation(owner, "p/Api.pick", "()V", file, 20, 30, scope);
        let overlapping = function_descriptor(owner, "(I)V", file, 10, 40, scope);
        let output = projected_calls(vec![descriptor.clone(), overlapping, call.clone()]);
        assert!(
            !output
                .iter()
                .any(|(fact, _)| fact["schema"] == DOCUMENTATION_CALL_SCHEMA)
        );
        assert!(
            output
                .iter()
                .any(|(fact, _)| fact["code"] == "KOTLIN_CALL_OWNER_NOT_UNIQUE")
        );

        let wrong_scope = function_descriptor(owner, "()V", file, 0, 100, ":/test");
        let wrong_scope_call = call_relation(owner, "p/Api.pick", "()V", file, 20, 30, ":/main");
        let output = projected_calls(vec![wrong_scope, wrong_scope_call]);
        assert!(
            !output
                .iter()
                .any(|(fact, _)| fact["schema"] == DOCUMENTATION_CALL_SCHEMA)
        );

        let mut partial = call_relation(owner, "p/Api.pick", "()V", file, 20, 30, scope);
        for field in [
            "receiverSelection",
            "receiverType",
            "argumentToParameter",
            "omittedDefaultParameterIndices",
        ] {
            partial.0.as_object_mut().unwrap().remove(field);
        }
        partial.0["attributeCoverage"] = json!("PARTIAL");
        partial.0["sourceRowHash"] = json!(format!("sha256:{}", "1".repeat(64)));
        let output = projected_calls(vec![descriptor, partial]);
        assert!(
            !output
                .iter()
                .any(|(fact, _)| fact["schema"] == DOCUMENTATION_CALL_SCHEMA)
        );
        assert!(
            output
                .iter()
                .any(|(fact, _)| fact["code"] == "KOTLIN_CALL_RELATION_PARTIAL")
        );

        let (mut partial_owner, _) = function_descriptor(owner, "()V", file, 0, 100, scope);
        for field in [
            "visibility",
            "effectiveVisibility",
            "exportBoundary",
            "modality",
            "isOverride",
            "returnType",
            "returnNullable",
            "parameterTypes",
            "typeParameters",
        ] {
            partial_owner.as_object_mut().unwrap().remove(field);
        }
        partial_owner["attributeCoverage"] = json!("PARTIAL");
        partial_owner["sourceRowHash"] = json!(format!("sha256:{}", "2".repeat(64)));
        let call = call_relation(owner, "p/Api.pick", "()V", file, 20, 30, scope);
        let output = projected_calls(vec![(partial_owner, "partial-owner".into()), call]);
        assert!(
            !output
                .iter()
                .any(|(fact, _)| fact["schema"] == DOCUMENTATION_CALL_SCHEMA)
        );
        assert!(
            output
                .iter()
                .any(|(fact, _)| fact["code"] == "KOTLIN_CALL_OWNER_NOT_UNIQUE")
        );
    }

    #[test]
    fn target_conflicts_and_target_resolution_boundaries_block_calls_but_cfg_does_not() {
        let owner = "p/caller";
        let file = "src/main/kotlin/p/Calls.kt";
        let scope = ":/main";
        let descriptor = function_descriptor(owner, "()V", file, 0, 100, scope);
        let first = call_relation(owner, "p/Api.pick", "()V", file, 20, 30, scope);
        let second = call_relation(owner, "p/Api.pick", "()I", file, 20, 30, scope);
        let output = projected_calls(vec![descriptor.clone(), first, second]);
        assert!(
            !output
                .iter()
                .any(|(fact, _)| fact["schema"] == DOCUMENTATION_CALL_SCHEMA)
        );
        assert!(
            output
                .iter()
                .any(|(fact, _)| fact["code"] == "KOTLIN_CALL_RELATION_CONFLICT")
        );

        let exact = call_relation(owner, "p/Api.pick", "()V", file, 20, 30, scope);
        let target_boundary = relation_boundary(
            "TARGET_IDENTITY",
            "UNRESOLVED_TARGET_JVM_DESCRIPTOR",
            owner,
            file,
            20,
            30,
            scope,
        );
        let output = projected_calls(vec![descriptor.clone(), exact.clone(), target_boundary]);
        assert!(
            !output
                .iter()
                .any(|(fact, _)| fact["schema"] == DOCUMENTATION_CALL_SCHEMA)
        );

        let cfg_boundary = relation_boundary(
            "ORDER_PROVENANCE",
            "NO_CFG_NODE_FOR_RELATION",
            owner,
            file,
            20,
            30,
            scope,
        );
        let output = projected_calls(vec![descriptor, exact, cfg_boundary]);
        assert!(
            output
                .iter()
                .any(|(fact, _)| fact["schema"] == DOCUMENTATION_CALL_SCHEMA)
        );
        assert!(
            output
                .iter()
                .any(|(fact, _)| fact["code"] == "NO_CFG_NODE_FOR_RELATION")
        );
        assert!(!output.iter().any(|(fact, _)| fact["kind"] == "FLOW"));
    }

    #[test]
    fn wide_target_resolution_boundaries_match_scope_owner_and_target_without_occurrences() {
        let owner = "p/caller";
        let file = "src/main/kotlin/p/Calls.kt";
        let scope = ":/main";
        let descriptor = function_descriptor(owner, "()V", file, 0, 100, scope);
        let exact = call_relation(owner, "p/Api.pick", "()V", file, 20, 30, scope);

        let unrelated_owner = wide_relation_boundary(
            "CALL_RESOLUTION",
            Some("p/other"),
            Some("p/Api.pick"),
            scope,
        );
        let output = projected_calls(vec![descriptor.clone(), exact.clone(), unrelated_owner]);
        assert!(
            output
                .iter()
                .any(|(fact, _)| fact["schema"] == DOCUMENTATION_CALL_SCHEMA)
        );

        let matching_owner = wide_relation_boundary("CALL_RESOLUTION", Some(owner), None, scope);
        let output = projected_calls(vec![descriptor.clone(), exact.clone(), matching_owner]);
        assert!(
            !output
                .iter()
                .any(|(fact, _)| fact["schema"] == DOCUMENTATION_CALL_SCHEMA)
        );

        let matching_target =
            wide_relation_boundary("TARGET_RESOLUTION", None, Some("p/Api.pick"), scope);
        let output = projected_calls(vec![descriptor.clone(), exact.clone(), matching_target]);
        assert!(
            !output
                .iter()
                .any(|(fact, _)| fact["schema"] == DOCUMENTATION_CALL_SCHEMA)
        );

        let broad = wide_relation_boundary("RELATION_RESOLUTION", None, None, scope);
        let output = projected_calls(vec![descriptor, exact, broad]);
        assert!(
            !output
                .iter()
                .any(|(fact, _)| fact["schema"] == DOCUMENTATION_CALL_SCHEMA)
        );
    }

    #[test]
    fn full_function_owner_can_derive_optional_jvm_descriptor_from_identity() {
        let owner = "p/caller";
        let file = "src/main/kotlin/p/Calls.kt";
        let scope = ":/main";
        let (mut descriptor, _) = function_descriptor(owner, "()V", file, 0, 100, scope);
        descriptor.as_object_mut().unwrap().remove("jvmDescriptor");
        let call = call_relation(owner, "p/Api.pick", "()V", file, 20, 30, scope);
        let output = projected_calls(vec![(descriptor, "owner-with-identity-only".into()), call]);
        let projected = output
            .iter()
            .find(|(fact, _)| fact["schema"] == DOCUMENTATION_CALL_SCHEMA)
            .unwrap();
        assert_eq!(projected.0["sourceJvmDescriptor"], "()V");
    }

    #[test]
    #[ignore = "launches the trusted Kotlin worker and Maven for Kotlin 1.9 documentation"]
    fn native_kotlin_19_maven_documentation() {
        use crate::{
            cas::CasStore, kotlin_adapter_v2::ProjectNativeKotlinWorkspace, repository_snapshot,
            state::StateAuthority,
        };
        let _guard = crate::worker::workspace_worker_test_lock();
        let private = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(private.path().join("v2")).unwrap();
        let cas = CasStore::open(&state).unwrap();
        let fixture = crate::worker::workspace_root().join("fixtures/durable-docs-kotlin");
        let (snapshot, _) = repository_snapshot::capture(&fixture, &cas).unwrap();
        let workspace =
            ProjectNativeKotlinWorkspace::prepare(&state, &cas, &snapshot, &[":/main".into()])
                .unwrap();
        let attempt = workspace
            .open_compilation_from_set(&state, ":/main", &"a".repeat(64), None)
            .unwrap();
        let (index, _, _) = attempt.analyze().unwrap();
        workspace.finish().unwrap();
        assert_eq!(index["k2Validated"], true, "{index}");
        assert_eq!(index["projectCompilerVersion"], "1.9.25");
        assert!(index["kotlinProjectSemantics"].is_object());
        let records = crate::kotlin_adapter_v2::translate_facts(&cas, &index).unwrap();
        let facts = records
            .into_iter()
            .map(|fact| {
                let lease = cas.read(&fact.payload, 64 * 1024 * 1024).unwrap();
                (
                    serde_json::from_slice(lease.bytes()).unwrap(),
                    fact.payload.digest,
                )
            })
            .collect();
        let service: Service = serde_json::from_value(json!({"schema":"codeclew-documentation-service/1.0", "id":"warehouse", "title":"Warehouse import", "repositoryId":"warehouse", "repository":"https://example.invalid/warehouse",
            "language":"kotlin", "profile":"kotlin-jvm-maven-analysis", "compilations":[":/main"], "targetRef":"main"})).unwrap();
        let source = "src/main/kotlin/example/ImportController.kt";
        let evidence = analysis::project(
            &service,
            &"b".repeat(40),
            &digest(&service).unwrap(),
            "DEVELOPMENT",
            "PARTIAL",
            project_facts(facts).unwrap(),
            &BTreeMap::from([(
                source.into(),
                std::fs::read_to_string(fixture.join(source)).unwrap(),
            )]),
            false,
        )
        .unwrap();
        assert_eq!(evidence.entrypoints.len(), 2, "{evidence:?}");
        assert!(
            evidence
                .entrypoints
                .iter()
                .any(|e| e.kind == "HTTP_ENDPOINT"
                    && e.trigger["paths"] == json!(["/imports/stock"]))
        );
        assert!(
            evidence
                .entrypoints
                .iter()
                .any(|e| e.kind == "KAFKA_LISTENER"
                    && e.trigger["configuration"]["topics"] == json!(["stock-import"]))
        );
        assert!(evidence.entrypoints.iter().all(|e| {
            !e.boundaries
                .contains(&"IMPLEMENTATION_BODY_UNAVAILABLE".into())
        }));
        assert!(
            evidence
                .observations
                .values()
                .any(|o| o.kind == "FLOW" && o.normalized["kind"] == "IF")
        );
        assert!(
            evidence
                .observations
                .values()
                .any(|o| o.kind == "FLOW" && o.normalized["kafka"]["topic"] == "stock-import")
        );
        assert!(
            evidence
                .boundaries
                .contains(&"KOTLIN_ANALYSIS_LANGUAGE_UPGRADED_FROM_1_9_TO_2_0".into()),
            "{:?}",
            evidence.boundaries
        );
        let files = BTreeMap::from([(
            source.to_owned(),
            std::fs::read_to_string(fixture.join(source)).unwrap(),
        )]);
        let mut syntax =
            crate::documentation::syntax::source_for_provider_test(&service, &evidence, &files);
        let roots = syntax.entrypoints.clone();
        crate::documentation::syntax::enrich(&mut syntax, Ok(evidence.clone())).unwrap();
        assert_eq!(syntax.entrypoints, roots);
        assert!(
            syntax
                .observations
                .values()
                .any(|o| o.kind == "SEMANTIC_SYMBOL" && o.normalized["fact"]["name"] == "publish"),
            "{:?}",
            syntax.boundaries
        );
        assert!(
            syntax
                .observations
                .values()
                .filter(|o| o.kind == "SEMANTIC_SYMBOL")
                .any(|o| o.normalized["boundaries"].as_array().is_some_and(|b| b
                    .iter()
                    .any(|v| v == "KOTLIN_ANALYSIS_LANGUAGE_UPGRADED_FROM_1_9_TO_2_0")))
        );
        let mut unavailable =
            crate::documentation::syntax::source_for_provider_test(&service, &evidence, &files);
        crate::documentation::syntax::enrich(
            &mut unavailable,
            Err(crate::documentation::invalid(
                "K2 unavailable in this fixture",
            )),
        )
        .unwrap();
        assert_eq!(unavailable.entrypoints, roots);
        assert!(
            !unavailable
                .observations
                .values()
                .any(|o| o.kind == "SEMANTIC_SYMBOL")
        );
    }

    fn chain(count: usize) -> check::Check {
        let mut services = BTreeMap::new();
        let mut interactions = BTreeMap::new();
        let target = "callable:org/springframework/kafka/core/KafkaTemplate.send#jvm:(Ljava/lang/String;Ljava/lang/Object;)Ljava/util/concurrent/CompletableFuture;";
        for index in 0..count {
            let id = format!("service{index}");
            let service = Service {
                schema: "codeclew-documentation-service/1.0".into(),
                id: id.clone(),
                title: id.clone(),
                repository_id: id.clone(),
                repository: format!("https://example.invalid/{id}"),
                language: "kotlin".into(),
                profile: "kotlin-jvm-maven-analysis".into(),
                source: None,
                modules: None,
                compilations: vec![":/main".into()],
                target_ref: "main".into(),
                source_link_template: None,
                contract_files: vec![],
                annotation_processor_paths: vec![],
            };
            store::validate_service(&service).unwrap();
            let symbol = "callable:example/Importer.importStock#jvm:()V";
            let declaration = json!({"schema":"declaration-descriptor/0.1", "symbolIdentity":symbol,
                "compilerCallableId":"example/Importer.importStock", "ownerIdentity":"class:example/Importer",
                "file":"Importer.kt", "startLine":1, "endLine":1, "jvmDescriptor":"()V"});
            let file = json!({"declarations":[{"documentationSymbol":symbol,"documentation":{
                "parameterTypes":[], "events":[{"kind":"CALL","target":target,"file":"Importer.kt","startLine":1,"endLine":1,
                    "kafka":{"topic":"stock-import"}}], "boundaries":[]}}]});
            let facts = vec![
                (declaration.clone(), digest(&declaration).unwrap()),
                (file.clone(), digest(&file).unwrap()),
            ];
            let projected = project_facts(facts).unwrap();
            let evidence = analysis::project(
                &service,
                &"a".repeat(40),
                &digest(&service).unwrap(),
                "DEVELOPMENT",
                "PARTIAL",
                projected,
                &BTreeMap::from([("Importer.kt".into(), "fun importStock() { send() }".into())]),
                false,
            )
            .unwrap();
            services.insert(id.clone(), evidence);
            if index + 1 < count {
                let interaction: Interaction = serde_json::from_value(json!({
                    "schema":"codeclew-documentation-interaction/1.0", "id":format!("next{index}"), "title":"Continue stock import",
                    "from":{"service":id,"selector":{"language":"kotlin","owner":"example.Importer","name":"importStock","parameterTypes":[]},"callSite":{"target":target}},
                    "to":{"service":format!("service{}",index+1),"selector":{"language":"kotlin","owner":"example.Importer","name":"importStock","parameterTypes":[]}},
                    "transport":{"kind":"kafka","topic":"stock-import"},"declaration":{"origin":"imported","rationale":"Synthetic declared event link"}
                })).unwrap();
                interactions.insert(interaction.id.clone(), interaction);
            }
        }
        let scenario: Scenario = serde_json::from_value(json!({"schema":"codeclew-documentation-process/1.0","process":{"scope":"Stock propagation","participants":["service0"],"trigger":"Stock update","outcomes":["Propagated stock"]},"id":"import","title":"Import stock",
            "summary":"Propagate stock changes.","root":{"service":"service0","selector":{"language":"kotlin","owner":"example.Importer","name":"importStock","parameterTypes":[]}},
            "interactions":interactions.keys().collect::<Vec<_>>(),"maxDepth":16,"maxNodes":64})).unwrap();
        check::assemble(
            "input".into(),
            services,
            BTreeMap::new(),
            &interactions,
            &BTreeMap::from([("import".into(), scenario)]),
        )
        .unwrap()
    }

    #[test]
    fn nine_kotlin_services_compose_without_an_artificial_participant_boundary() {
        let checked = chain(8);
        let scenario = &checked.scenarios["import"];
        assert_eq!(
            scenario
                .steps
                .iter()
                .filter(|s| s.kind == "DECLARED_KAFKA_TRANSITION")
                .count(),
            7
        );
        assert_eq!(
            scenario
                .steps
                .iter()
                .map(|s| &s.service)
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            8
        );
        assert!(!scenario.truncated);
        assert_eq!(
            checked.interactions["next0"].topic["caller"]["status"],
            "MATCH"
        );
        let expanded_check = chain(9);
        let expanded = &expanded_check.scenarios["import"];
        assert_eq!(
            expanded
                .steps
                .iter()
                .filter(|step| step.kind == "DECLARED_KAFKA_TRANSITION")
                .count(),
            8
        );
        assert_eq!(
            expanded
                .steps
                .iter()
                .map(|step| &step.service)
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            9
        );
        assert!(!expanded.truncated);
    }

    #[test]
    fn explanation_requires_step_evidence_and_supports_eight_services_plus_client() {
        let checked = chain(8);
        let step = &checked.scenarios["import"].steps[0];
        let mut narrative: Narrative = serde_json::from_value(json!({
            "schema":"codeclew-documentation-narrative/1.3", "subject":"scenario:import", "contextDigest":checked.context_digest,
            "operations":[{"id":"import","title":"Import stock","summary":{"id":"summary","text":"Propagate stock changes.","dependencyIds":step.dependency_ids,"sourceIds":step.source_ids},
                "participants":(0..8).map(|i|json!({"id":format!("p{i}"),"label":format!("Service {i}"),"service":format!("service{i}")})).chain([json!({"id":"client","label":"Warehouse operator","service":null})]).collect::<Vec<_>>(),
                "overviewDiagram":{"nodes":[{"id":"accept","text":"Accept the stock batch","participant":"p0","column":0,"row":0,"eventIds":["send"]}],"edges":[]},
                "events":[{"id":"send","kind":"message","text":"Accept the warehouse stock batch","from":"client","to":"p0","dependencyIds":step.dependency_ids,"sourceIds":step.source_ids}],
                "explanation":[{"id":"acceptance","text":"The operator submits the available quantity for the warehouse. The importing service accepts the batch and prepares its propagation to the next service.","eventIds":["send"],"dependencyIds":step.dependency_ids,"sourceIds":step.source_ids}]}]
        })).unwrap();
        render::validate(&narrative, &checked).unwrap();
        let bindings = render::make_bindings(
            &checked,
            BTreeMap::from([(narrative.subject.clone(), narrative.clone())]),
        )
        .unwrap();
        assert!(
            bindings
                .fragments
                .contains_key("scenario:import/import/acceptance")
        );
        narrative.operations[0].explanation[0].detail = true;
        narrative.operations[0].interface_contracts = vec![InterfaceContract {
            id: "stock-message".into(),
            title: "Stock import message".into(),
            kind: "kafka".into(),
            rows: vec![InterfaceContractRow {
                id: "stock-topic".into(),
                label: "Topic".into(),
                value: "stock-import".into(),
                dependency_ids: step.dependency_ids.clone(),
                source_ids: step.source_ids.clone(),
            }],
            boundaries: vec!["Broker delivery is not established.".into()],
        }];
        render::validate(&narrative, &checked).unwrap();
        let contract_bindings = render::make_bindings(
            &checked,
            BTreeMap::from([(narrative.subject.clone(), narrative.clone())]),
        )
        .unwrap();
        assert!(
            contract_bindings
                .fragments
                .contains_key("scenario:import/import/stock-topic")
        );
        let mut overview = narrative.clone();
        overview.operations[0].overview_diagram = None;
        render::validate(&overview, &checked).unwrap();
        assert!(render::mermaid(&overview.operations[0]).starts_with("sequenceDiagram"));
        overview.operations[0].overview_diagram = Some(serde_json::from_value(json!({
            "nodes":[{"id":"accept","text":"Accept stock","participant":"p0","column":0,"row":0,"eventIds":["send"]},
                {"id":"received","text":"Batch received","participant":"p0","column":1,"row":0,"eventIds":["send"]}],
            "edges":[{"id":"accepted","from":"accept","to":"received","text":"accepted","eventIds":["send"]}]
        })).unwrap());
        render::validate(&overview, &checked).unwrap();
        assert!(render::mermaid(&overview.operations[0]).starts_with("flowchart LR"));
        let overview_bindings = render::make_bindings(
            &checked,
            BTreeMap::from([(overview.subject.clone(), overview.clone())]),
        )
        .unwrap();
        assert!(
            overview_bindings
                .fragments
                .contains_key("scenario:import/import/overview/accept")
        );
        let valid = overview.operations[0].overview_diagram.clone();
        overview.operations[0]
            .overview_diagram
            .as_mut()
            .unwrap()
            .nodes[0]
            .event_ids = vec!["missing".into()];
        assert!(render::validate(&overview, &checked).is_err());
        overview.operations[0].overview_diagram = valid.clone();
        overview.operations[0]
            .overview_diagram
            .as_mut()
            .unwrap()
            .nodes[1]
            .column = 0;
        assert!(render::validate(&overview, &checked).is_err());
        overview.operations[0].overview_diagram = valid.clone();
        overview.operations[0]
            .overview_diagram
            .as_mut()
            .unwrap()
            .nodes[1]
            .participant = "p1".into();
        assert!(render::validate(&overview, &checked).is_err());
        overview.operations[0].overview_diagram = valid.clone();
        overview.operations[0]
            .overview_diagram
            .as_mut()
            .unwrap()
            .nodes = vec![valid.as_ref().unwrap().nodes[0].clone(); 13];
        assert!(render::validate(&overview, &checked).is_err());
        let mut changed = checked.clone();
        changed
            .dependencies
            .get_mut(&step.dependency_ids[0])
            .unwrap()
            .digest = "changed".into();
        let freshness =
            crate::documentation::bindings::freshness(Some(&contract_bindings), &changed);
        assert!(
            freshness["affected"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["fragment"] == "scenario:import/import/stock-topic")
        );
        let freshness =
            crate::documentation::bindings::freshness(Some(&overview_bindings), &changed);
        assert!(
            freshness["affected"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["fragment"] == "scenario:import/import/overview/accept")
        );
        narrative.operations[0].interface_contracts[0].rows[0].source_ids = vec!["unbound".into()];
        assert!(render::validate(&narrative, &checked).is_err());
        narrative.operations[0].interface_contracts.clear();
        narrative.operations[0].explanation[0].event_ids = vec!["unknown".into()];
        assert!(render::validate(&narrative, &checked).is_err());
        narrative.operations[0].explanation.clear();
        assert!(render::validate(&narrative, &checked).is_err());
        overview.operations[0].overview_diagram = valid;
        render::validate(&overview, &checked).unwrap();
        for version in ["1.0", "1.1", "1.2"] {
            let mut obsolete = overview.clone();
            obsolete.schema = format!("codeclew-documentation-narrative/{version}");
            assert!(
                render::validate(&obsolete, &checked)
                    .unwrap_err()
                    .message
                    .contains("only codeclew-documentation-narrative/1.3")
            );
        }
    }
}
