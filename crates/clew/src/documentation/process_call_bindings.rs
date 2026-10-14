//! Project admitted retained K2 call evidence; never resolve a call from source text.
use super::{
    digest, invalid,
    model::{Observation, ServiceEvidence},
};
use crate::{canonical, error::ClewError};
use serde_json::{Value, json};

pub(super) const POLICY: &str = "retained-k2-process-call-bindings/0.1";

/// A claimed K2 contract still has to pass every check in `project`.
pub(super) fn claims_k2_contract(relation: &Observation) -> bool {
    relation.normalized["schema"] == "codeclew-kotlin-documentation-call/1.0"
        || relation.normalized["provider"] == "K2_FIR"
}

/// Caller and target identities belong to the same retained compilation scope.
/// An exact compiler-selected target is not an exact runtime dispatch receiver.
pub(super) fn project(
    evidence: &ServiceEvidence,
    caller: &Observation,
    relation: &Observation,
    candidates: &[&Observation],
    unscoped_target: bool,
    other_scope_target: bool,
) -> Result<Value, ClewError> {
    let fact = &relation.normalized;
    if relation.kind != "CALL_RELATION"
        || relation.service != evidence.service
        || caller.service != evidence.service
        || relation.symbol != caller.symbol
        || fact["sourceIdentity"] != caller.symbol
        || fact["scope"] != caller.normalized["scope"]
        || fact["scope"].as_str().is_none_or(str::is_empty)
        || fact["schema"] != "codeclew-kotlin-documentation-call/1.0"
        || fact["kind"] != "RELATION"
        || fact["relationKind"] != "CALLS"
        || fact["resolution"] != "COMPILER_EXACT"
        || fact["compilerResolution"] != "PROVEN"
        || fact["provider"] != "K2_FIR"
        || fact["compilerSchema"] != "declaration-relation/0.1"
        || fact["sourceProvenance"] != "COMPILER_UTF16_RANGE_TO_UTF8_BYTES"
        || digest(fact)? != relation.digest
    {
        return Err(invalid(
            "process call does not bind admitted exact caller evidence",
        ));
    }
    let identity = |prefix: &str| -> Result<String, ClewError> {
        let callable = fact[format!("{prefix}CompilerCallableId")]
            .as_str()
            .ok_or_else(|| invalid("process call lacks compiler callable identity"))?;
        let descriptor = fact[format!("{prefix}JvmDescriptor")]
            .as_str()
            .ok_or_else(|| invalid("process call lacks JVM descriptor"))?;
        let identity = format!("callable:{callable}#jvm:{descriptor}");
        crate::semantic_validation::validate_kotlin_full_symbol_identity(&identity)?;
        Ok(identity)
    };
    let source_identity = identity("source")?;
    let target_identity = identity("target")?;
    if fact["sourceIdentity"] != source_identity || fact["targetIdentity"] != target_identity {
        return Err(invalid(
            "process call descriptor identity differs from its selected symbol",
        ));
    }
    let site = &fact["callSite"];
    let [source_id] = relation.source_ids.as_slice() else {
        return Err(invalid(
            "process call needs exactly one retained source occurrence",
        ));
    };
    let source = evidence
        .sources
        .get(source_id)
        .ok_or_else(|| invalid("process call source is missing"))?;
    let start = site["byteStart"]
        .as_u64()
        .ok_or_else(|| invalid("process call start is missing"))?;
    let end = site["byteEnd"]
        .as_u64()
        .ok_or_else(|| invalid("process call end is missing"))?;
    if source.service != evidence.service
        || source.revision != evidence.revision
        || site["sourceId"] != *source_id
        || site["sourceStatus"] != "SOURCE_RETAINED"
        || site["file"] != source.file
        || site["startLine"] != source.start_line
        || site["endLine"] != source.end_line
        || site["sourceDigest"] != source.text_digest
        || site["evidenceDigest"] != source.evidence_digest
        || fact["evidenceBinding"] != source.evidence_digest
        || source.text_digest != canonical::hash_bytes(source.text.as_bytes())
        || end.checked_sub(start) != Some(source.text.len() as u64)
        || source.text.is_empty()
    {
        return Err(invalid(
            "process call source revision, range or digest differs",
        ));
    }
    if candidates.iter().any(|method| {
        method.service != evidence.service
            || !super::process_graph::callable_observation(method)
            || method.symbol != target_identity
            || method.normalized["scope"] != fact["scope"]
    }) {
        return Err(invalid(
            "process target index differs from exact call identity",
        ));
    }
    let (target_status, target_method) = match candidates {
        [method] if !unscoped_target => ("SAME_SCOPE_UNIQUE", Some(method.id.as_str())),
        [_] => ("TARGET_SCOPE_ASSOCIATION_AMBIGUOUS", None),
        [] if other_scope_target => ("TARGET_ONLY_IN_OTHER_SCOPE", None),
        [] if unscoped_target => ("TARGET_SCOPE_UNAVAILABLE", None),
        [] => ("EXTERNAL_OR_UNRETAINED_TARGET", None),
        _ => ("SAME_SCOPE_AMBIGUOUS", None),
    };
    let mut arguments = Vec::new();
    let mut defaults = json!([]);
    let argument_status = if let Some(mapping) = fact.get("argumentBindings") {
        if mapping["schema"] != "codeclew-call-argument-bindings/1.0" {
            return Err(invalid("unsupported process argument binding schema"));
        }
        crate::semantic_validation::validate_kotlin_call_argument_bindings(
            start,
            end,
            fact["targetJvmDescriptor"].as_str().unwrap(),
            &mapping["argumentToParameter"],
            &mapping["omittedDefaultParameterIndices"],
        )?;
        for row in mapping["argumentToParameter"].as_array().unwrap() {
            let local_start = usize::try_from(row["argumentStart"].as_u64().unwrap() - start)
                .map_err(|_| invalid("process argument start exceeds addressable bytes"))?;
            let local_end = usize::try_from(row["argumentEnd"].as_u64().unwrap() - start)
                .map_err(|_| invalid("process argument end exceeds addressable bytes"))?;
            let text = source
                .text
                .get(local_start..local_end)
                .ok_or_else(|| invalid("process argument splits retained UTF-8 source"))?;
            let mut row = row.clone();
            row["expression"] = json!(text);
            row["sourceId"] = json!(source_id);
            row["valueKnowledge"] = json!("SOURCE_EXPRESSION_NOT_EVALUATED_VALUE");
            arguments.push(row);
        }
        defaults = mapping["omittedDefaultParameterIndices"].clone();
        "COMPILER_PARAMETER_MAPPING"
    } else {
        "UNKNOWN"
    };
    Ok(json!({
        "schema":"codeclew-process-call-binding/0.1", "policy":POLICY,
        "relationId":relation.id,"factDigest":relation.digest,"callerMethodId":caller.id,
        "targetIdentity":target_identity,"targetMethodId":target_method,"targetStatus":target_status,
        "candidateTargetMethodIds":candidates.iter().map(|m| &m.id).collect::<Vec<_>>(),
        "targetAuthority":"RETAINED_K2_COMPILER_SELECTED_SYMBOL_NOT_RUNTIME_DISPATCH",
        "callSite":site,"sourceIds":relation.source_ids,"sourceAuthority":source.authority,
        "argumentStatus":argument_status,"arguments":arguments,
        "omittedDefaultParameterIndices":defaults,"defaultValues":"UNKNOWN",
        "receiver":{"selection":fact["receiverEvidence"]["selection"],"type":fact["receiverEvidence"]["type"],"objectIdentity":"UNKNOWN"},
        "returnTransfer":{"status":"UNKNOWN","declaredType":fact["declaredResultType"],"requiredEvidence":"QUALIFIED_VALUE_RETURN_EQUATIONS_AND_CALLER_RESULT_BINDING"},
        "failureTransfer":{"status":"UNKNOWN","requiredEvidence":"QUALIFIED_EXCEPTION_AND_EFFECT_TRANSFER"},
        "callerContinuation":"NOT_ESTABLISHED_BY_CALL_BINDING"
    }))
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    pub(crate) fn fixture() -> (ServiceEvidence, Observation, Observation, Observation) {
        let caller_symbol = "callable:example.Api.start#jvm:()V";
        let target_symbol =
            "callable:example.Api.pick#jvm:(ILjava/lang/String;Z)Ljava/lang/String;";
        let text = "api.pick(text = \"é\", count = 7)";
        let start = 100usize;
        let expression_start = start + text.find("text =").unwrap();
        let number_start = start + text.find("count =").unwrap();
        let caller: Observation = serde_json::from_value(json!({"id":"caller","kind":"SYMBOL","service":"svc","symbol":caller_symbol,
            "digest":"fixture-caller","sourceIds":[],"normalized":{"declarationKind":"METHOD","scope":":main"}})).unwrap();
        let target: Observation = serde_json::from_value(json!({"id":"callee","kind":"SYMBOL","service":"svc","symbol":target_symbol,
            "digest":"fixture-callee","sourceIds":[],"normalized":{"declarationKind":"METHOD","scope":":main"}})).unwrap();
        let source = json!({"id":"site","service":"svc","revision":"rev","file":"Example.kt","startLine":1,"endLine":1,
            "text":text,"textDigest":canonical::hash_bytes(text.as_bytes()),"evidenceDigest":"fixture-binding","authority":"SYNTHETIC_COMPILER_FACT_FIXTURE"});
        let fact = json!({"schema":"codeclew-kotlin-documentation-call/1.0","kind":"RELATION","relationKind":"CALLS","scope":":main",
            "sourceIdentity":caller_symbol,"targetIdentity":target_symbol,"resolution":"COMPILER_EXACT","compilerResolution":"PROVEN","provider":"K2_FIR",
            "compilerSchema":"declaration-relation/0.1","sourceProvenance":"COMPILER_UTF16_RANGE_TO_UTF8_BYTES","evidenceBinding":"fixture-binding",
            "sourceCompilerCallableId":"example.Api.start","sourceJvmDescriptor":"()V","targetCompilerCallableId":"example.Api.pick","targetJvmDescriptor":"(ILjava/lang/String;Z)Ljava/lang/String;",
            "callSite":{"file":"Example.kt","startLine":1,"endLine":1,"byteStart":start,"byteEnd":start+text.len(),"sourceId":"site",
                "sourceStatus":"SOURCE_RETAINED","sourceDigest":source["textDigest"],"evidenceDigest":"fixture-binding"},
            "argumentBindings":{"schema":"codeclew-call-argument-bindings/1.0","omittedDefaultParameterIndices":[2],"argumentToParameter":[
                {"argumentStart":expression_start,"argumentEnd":number_start-2,"argumentName":"text","argumentType":"kotlin.String","parameter":"text","parameterIndex":1,"parameterType":"kotlin.String"},
                {"argumentStart":number_start,"argumentEnd":start+text.len()-1,"argumentName":"count","argumentType":"kotlin.Int","parameter":"count","parameterIndex":0,"parameterType":"kotlin.Int"}]},
            "receiverEvidence":{"selection":"EXPLICIT","type":"example.Api","objectIdentity":"UNKNOWN"},"declaredResultType":"kotlin.String"});
        let relation = Observation {
            id: "call-site".into(),
            kind: "CALL_RELATION".into(),
            service: "svc".into(),
            symbol: caller_symbol.into(),
            digest: digest(&fact).unwrap(),
            normalized: fact,
            source_ids: vec!["site".into()],
        };
        let mut evidence: ServiceEvidence = serde_json::from_value(json!({"schema":"test","service":"svc","revision":"rev","serviceDigest":"fixture",
            "extractor":"SYNTHETIC_K2_CALL_FIXTURE","runtimeMode":"TEST","coverage":"PARTIAL","boundaries":[],"entrypoints":[],"observations":{},"sources":{"site":source},"contracts":{}})).unwrap();
        for observation in [&caller, &target, &relation] {
            evidence
                .observations
                .insert(observation.id.clone(), observation.clone());
        }
        (evidence, caller, target, relation)
    }

    #[test]
    fn named_actuals_keep_source_order_and_exact_formal_slots_with_unknown_runtime_values() {
        let (evidence, caller, target, relation) = fixture();
        let projected = project(&evidence, &caller, &relation, &[&target], false, false).unwrap();
        assert_eq!(projected["targetMethodId"], "callee");
        assert_eq!(projected["arguments"][0]["parameterIndex"], 1);
        assert_eq!(projected["arguments"][0]["expression"], "text = \"é\"");
        assert_eq!(projected["arguments"][1]["expression"], "count = 7");
        assert_eq!(projected["omittedDefaultParameterIndices"], json!([2]));
        assert_eq!(projected["defaultValues"], "UNKNOWN");
        assert_eq!(projected["receiver"]["objectIdentity"], "UNKNOWN");
        assert_eq!(projected["returnTransfer"]["status"], "UNKNOWN");
        assert_eq!(projected["failureTransfer"]["status"], "UNKNOWN");
    }

    #[test]
    fn absent_mapping_is_unknown_and_ambiguous_or_foreign_scope_targets_are_not_followed() {
        let (evidence, caller, target, mut relation) = fixture();
        relation
            .normalized
            .as_object_mut()
            .unwrap()
            .remove("argumentBindings");
        relation.digest = digest(&relation.normalized).unwrap();
        let legacy = project(&evidence, &caller, &relation, &[&target], false, false).unwrap();
        assert_eq!(legacy["argumentStatus"], "UNKNOWN");
        assert_eq!(legacy["arguments"], json!([]));
        for (candidates, unscoped, other, status) in [
            (vec![&target, &target], false, false, "SAME_SCOPE_AMBIGUOUS"),
            (
                vec![&target],
                true,
                false,
                "TARGET_SCOPE_ASSOCIATION_AMBIGUOUS",
            ),
            (vec![], false, true, "TARGET_ONLY_IN_OTHER_SCOPE"),
            (vec![], true, false, "TARGET_SCOPE_UNAVAILABLE"),
            (vec![], false, false, "EXTERNAL_OR_UNRETAINED_TARGET"),
        ] {
            let p = project(&evidence, &caller, &relation, &candidates, unscoped, other).unwrap();
            assert_eq!(p["targetStatus"], status);
            assert_eq!(p["targetMethodId"], Value::Null);
        }
    }

    #[test]
    fn changed_bytes_overloads_bindings_and_split_utf8_fail_closed() {
        let (evidence, caller, target, relation) = fixture();
        let mut changed = evidence.clone();
        changed.sources.get_mut("site").unwrap().text.push('!');
        assert!(project(&changed, &caller, &relation, &[&target], false, false).is_err());
        changed = evidence.clone();
        changed.sources.get_mut("site").unwrap().revision = "other-revision".into();
        assert!(project(&changed, &caller, &relation, &[&target], false, false).is_err());
        let mut changed_range = relation.clone();
        changed_range.normalized["callSite"]["byteEnd"] = json!(1);
        changed_range.digest = digest(&changed_range.normalized).unwrap();
        assert!(project(&evidence, &caller, &changed_range, &[&target], false, false).is_err());
        for pointer in [
            "/targetJvmDescriptor",
            "/callSite/sourceDigest",
            "/callSite/evidenceDigest",
            "/sourceIdentity",
            "/scope",
        ] {
            let mut altered = relation.clone();
            *altered.normalized.pointer_mut(pointer).unwrap() = json!("different");
            altered.digest = digest(&altered.normalized).unwrap();
            assert!(
                project(&evidence, &caller, &altered, &[&target], false, false).is_err(),
                "{pointer}"
            );
        }
        let mut split = relation.clone();
        let text = &evidence.sources["site"].text;
        split.normalized["argumentBindings"]["argumentToParameter"][0]["argumentEnd"] =
            json!(100 + text.find('é').unwrap() + 1);
        split.digest = digest(&split.normalized).unwrap();
        assert!(project(&evidence, &caller, &split, &[&target], false, false).is_err());
        let mut wrong_overload = target;
        wrong_overload.symbol = "callable:example.Api.pick#jvm:(I)Ljava/lang/String;".into();
        assert!(
            project(
                &evidence,
                &caller,
                &relation,
                &[&wrong_overload],
                false,
                false
            )
            .is_err()
        );
    }
}
