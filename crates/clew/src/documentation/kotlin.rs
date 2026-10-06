//! Project retained K2 descriptors and compiler-linked PSI flow into documentation.
use super::{digest, invalid};
use crate::error::ClewError;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

const DOCUMENTATION_CALL_SCHEMA: &str = "codeclew-kotlin-documentation-call/1.0";

type CallOccurrence = (String, String, u64, u64);

#[derive(Clone)]
struct FunctionDescriptor {
    symbol: String,
    compiler_callable_id: String,
    jvm_descriptor: String,
    start: u64,
    end: u64,
}

struct ProjectedCall {
    occurrence: CallOccurrence,
    fact: Value,
    binding: String,
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

fn exact_call_fact(relation: &Value, owner: &FunctionDescriptor, binding: &str) -> Value {
    json!({
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
    })
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
    for (fact, _) in &facts {
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
    for (fact, binding) in &facts {
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
                fact: exact_call_fact(fact, owners[0], binding),
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
    if !output.iter().any(|(fact, _)| fact["kind"] == "DECLARATION") {
        return Err(ClewError::new(
            crate::error::ErrorCode::IncompleteSemanticAnalysis,
            "Kotlin compiler declarations are unavailable; resolve compilation dependencies before documenting behavior",
        ));
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::documentation::{analysis, check, model::*, render, store};

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
