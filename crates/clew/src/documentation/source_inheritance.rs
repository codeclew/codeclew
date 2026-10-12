//! Bounded projection of captured Kotlin interface mappings onto declared overrides.
//! This is source evidence, never compiler override resolution or runtime registration.
use super::{invalid, model::ServiceEvidence, syntax};
use crate::error::ClewError;
use clew_facts::SourceAnnotationFacts;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn apply(e: &mut ServiceEvidence) -> Result<(), ClewError> {
    let mut types: BTreeMap<String, Vec<_>> = BTreeMap::new();
    let mut methods: BTreeMap<String, Vec<_>> = BTreeMap::new();
    let mut annotations = BTreeMap::new();
    let parse_errors: BTreeSet<_> = e
        .boundaries
        .iter()
        .filter_map(|b| b.strip_prefix("PARSE_ERROR:").map(str::to_owned))
        .collect();
    for observation in e.observations.values() {
        if observation.source_ids.iter().any(|id| {
            e.sources
                .get(id)
                .is_some_and(|s| parse_errors.contains(&s.file))
        }) {
            continue;
        }
        let meta = &observation.normalized["sourceInheritance"];
        if observation.kind == "SYMBOL" {
            match meta["kind"].as_str() {
                Some("CLASS" | "INTERFACE") => {
                    if let Some(name) = meta["qualifiedName"].as_str() {
                        types
                            .entry(name.to_owned())
                            .or_default()
                            .push(observation.clone());
                    }
                }
                Some("METHOD") => {
                    if let Some(owner) = meta["owner"].as_str() {
                        methods
                            .entry(owner.to_owned())
                            .or_default()
                            .push(observation.clone());
                    }
                }
                _ => {}
            }
        } else if observation.kind == "SOURCE_ANNOTATIONS" {
            annotations.insert(observation.symbol.clone(), observation.clone());
        }
    }
    for index in 0..e.entrypoints.len() {
        let entry = &e.entrypoints[index];
        if entry.kind != "SOURCE_DECLARATION" {
            continue;
        }
        let Some(root_method) = entry
            .dependency_ids
            .iter()
            .filter_map(|id| e.observations.get(id))
            .find(|o| o.kind == "SYMBOL")
        else {
            continue;
        };
        let signature = &root_method.normalized["sourceInheritance"];
        if signature["overrideDeclared"] != true {
            continue;
        }
        let Some(root_annotation) = annotations.get(&entry.symbol) else {
            continue;
        };
        let mut facts: SourceAnnotationFacts =
            serde_json::from_value(root_annotation.normalized["facts"].clone())
                .map_err(|err| invalid(err.to_string()))?;
        if !facts.owners.iter().flat_map(|o| &o.annotations).any(|a| {
            matches!(
                a.qualified_name.as_deref(),
                Some(
                    "org.springframework.web.bind.annotation.RestController"
                        | "org.springframework.stereotype.Controller"
                )
            )
        }) {
            continue;
        }
        let Some(root_types) = signature["owner"]
            .as_str()
            .and_then(|owner| types.get(owner))
        else {
            continue;
        };
        if root_types.len() != 1 {
            continue;
        }
        let root_type = &root_types[0];
        let bases = root_type.normalized["sourceInheritance"]["supertypes"].as_array();
        let Some(bases) = bases else {
            continue;
        };
        if bases.is_empty() {
            continue;
        }
        // No guessing through missing, generic, ambiguous or transitive supertypes.
        let mut unresolved = false;
        let mut matches = Vec::new();
        for base in bases {
            let Some(name) = base.as_str() else {
                unresolved = true;
                continue;
            };
            let Some(base_types) = types.get(name) else {
                unresolved = true;
                continue;
            };
            if base_types.len() != 1 {
                unresolved = true;
                continue;
            }
            let base_type = &base_types[0];
            let meta = &base_type.normalized["sourceInheritance"];
            if meta["kind"] != "INTERFACE"
                || meta["generic"] == true
                || meta["supertypes"].as_array().is_none_or(|v| !v.is_empty())
            {
                unresolved = true;
                continue;
            }
            if let Some(candidates) = methods.get(name) {
                for method in candidates {
                    let meta = &method.normalized["sourceInheritance"];
                    if meta["name"] == signature["name"]
                        && meta["parameterTypes"] == signature["parameterTypes"]
                    {
                        matches.push((base_type, method));
                    }
                }
            }
        }
        if unresolved || matches.len() != 1 {
            let boundary = if unresolved {
                "SOURCE_HTTP_INTERFACE_NOT_CAPTURED_OR_UNRESOLVED"
            } else {
                "SOURCE_HTTP_OVERRIDE_SIGNATURE_AMBIGUOUS_OR_UNMATCHED"
            };
            e.entrypoints[index].boundaries.push(boundary.into());
            e.boundaries.push(boundary.into());
            continue;
        }
        let (base_type, base_method) = matches[0];
        let Some(base_annotation) = annotations.get(&base_method.symbol) else {
            continue;
        };
        let inherited: SourceAnnotationFacts =
            serde_json::from_value(base_annotation.normalized["facts"].clone())
                .map_err(|err| invalid(err.to_string()))?;
        // A root method annotation has precedence; do not merge competing handler declarations.
        if !facts.declaration.annotations.is_empty() {
            continue;
        }
        facts.declaration = inherited.declaration;
        facts.owners.extend(inherited.owners);
        facts.imports.extend(inherited.imports);
        facts.imports.sort();
        facts.imports.dedup();
        let boundary = "SOURCE_OVERRIDE_SIGNATURE_NOT_COMPILER_VERIFIED";
        facts.boundaries.push(boundary.into());
        let Ok(metadata) = clew_framework_spring::analyze_source(&facts) else {
            e.entrypoints[index]
                .boundaries
                .push("SOURCE_HTTP_INHERITED_ANNOTATION_QUALIFICATION_AMBIGUOUS".into());
            continue;
        };
        if metadata.entries.len() != 1
            || metadata.entries[0].kind != "HTTP_ENDPOINT"
            || metadata.entries[0].controller != Some(true)
        {
            continue;
        }
        if metadata
            .boundaries
            .iter()
            .any(|b| b == "MULTIPLE_REQUEST_MAPPINGS_ON_ELEMENT")
        {
            continue;
        }
        let dependencies = vec![
            root_method.id.clone(),
            root_type.id.clone(),
            root_annotation.id.clone(),
            base_method.id.clone(),
            base_type.id.clone(),
            base_annotation.id.clone(),
        ];
        let mut sources: Vec<_> = dependencies
            .iter()
            .flat_map(|id| e.observations[id].source_ids.clone())
            .collect();
        sources.sort();
        sources.dedup();
        let symbol = entry.symbol.clone();
        let id = syntax::observe(
            e,
            "SOURCE_INHERITED_HTTP_MAPPING",
            &symbol,
            json!({"authority":"SOURCE_DECLARED_INHERITANCE","dependencies":dependencies,"projectionInput":facts,"framework":metadata,"boundaries":[boundary]}),
            sources.clone(),
        )?;
        let entry = &mut e.entrypoints[index];
        entry.kind = "HTTP_ENDPOINT".into();
        entry.trigger = crate::spring_entrypoints::describe_trigger(&metadata.entries[0]);
        entry.trigger["authority"] = json!("FRAMEWORK_DERIVED_SOURCE");
        entry.trigger["inheritance"] = json!({"authority":"SOURCE_DECLARED_INHERITANCE","interfaceSymbol":base_method.symbol,"dependency":id});
        entry.source_ids.extend(sources);
        entry.source_ids.sort();
        entry.source_ids.dedup();
        entry.dependency_ids.extend(dependencies);
        entry.dependency_ids.push(id);
        entry.dependency_ids.sort();
        entry.dependency_ids.dedup();
        entry.boundaries.extend(metadata.boundaries);
        entry
            .boundaries
            .retain(|b| b != "TRIGGER_NOT_FRAMEWORK_RESOLVED");
        entry.boundaries.push(boundary.into());
        entry.boundaries.sort();
        entry.boundaries.dedup();
        e.boundaries.push(boundary.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::{analysis, model::Service};
    use super::*;
    const API: &str = "package api\nimport org.springframework.web.bind.annotation.PostMapping\nimport org.springframework.web.bind.annotation.RequestMapping\n@RequestMapping(\"/v1\")\ninterface DealApi {\n @PostMapping(\"/deal\")\n fun getDeal(source: String): String\n}\n";
    const CONTROLLER: &str = "package app\nimport api.DealApi\nimport org.springframework.web.bind.annotation.RestController\n@RestController class DealController : DealApi { override fun getDeal(source: String): String = source\n fun helper(): String = \"helper\"\n}\n";
    fn evidence(files: &[(&str, &str)]) -> ServiceEvidence {
        let service: Service = serde_json::from_value(json!({"schema":"codeclew-documentation-service/1.0","id":"orders","title":"Orders","repositoryId":"orders","repository":"https://example.invalid/orders","language":"kotlin","profile":"source-syntax","targetRef":"HEAD","source":{"roots":["."],"dialect":"2.3"}})).unwrap();
        let empty: ServiceEvidence = serde_json::from_value(json!({"schema":"codeclew-documentation-service-evidence/1.0","service":"orders","revision":"a".repeat(40),"serviceDigest":"digest","extractor":"source-syntax","runtimeMode":"COMMITTED_SOURCE_NO_BUILD","coverage":"SYNTAX","boundaries":[],"entrypoints":[],"observations":{},"sources":{},"contracts":{}})).unwrap();
        syntax::source_for_provider_test(
            &service,
            &empty,
            &files
                .iter()
                .map(|(p, t)| (p.to_string(), t.to_string()))
                .collect(),
        )
    }
    #[test]
    fn captured_interface_routes_belong_to_controller_and_keep_all_evidence() {
        let e = evidence(&[("DealApi.kt", API), ("DealController.kt", CONTROLLER)]);
        let public: Vec<_> = e
            .entrypoints
            .iter()
            .filter(|entry| super::super::process_candidates::public_boundary(entry))
            .collect();
        assert_eq!(public.len(), 1, "{:#?}", e.entrypoints);
        assert_eq!(
            public[0].trigger["paths"],
            json!(["/v1/deal"]),
            "{:#?}",
            e.observations
                .values()
                .filter(|o| o.kind == "SOURCE_INHERITED_HTTP_MAPPING")
                .collect::<Vec<_>>()
        );
        assert_eq!(public[0].trigger["methods"], json!(["POST"]));
        assert_eq!(e.entrypoints.len(), 3);
        assert_eq!(e.coverage, "SYNTAX");
        assert!(
            public[0]
                .boundaries
                .iter()
                .any(|b| b == "SOURCE_OVERRIDE_SIGNATURE_NOT_COMPILER_VERIFIED")
        );
        assert!(
            public[0]
                .source_ids
                .iter()
                .any(|id| e.sources[id].file == "DealApi.kt")
        );
        assert!(
            public[0]
                .source_ids
                .iter()
                .any(|id| e.sources[id].file == "DealController.kt")
        );
        analysis::verify_evidence(&e).unwrap();
    }
    #[test]
    fn missing_interface_and_mismatched_signature_do_not_invent_routes() {
        for files in [
            vec![("DealController.kt", CONTROLLER)],
            vec![
                ("DealApi.kt", API),
                (
                    "DealController.kt",
                    &CONTROLLER.replace("source: String", "source: Long"),
                ),
            ],
        ] {
            let e = evidence(&files);
            assert!(
                !e.entrypoints
                    .iter()
                    .any(super::super::process_candidates::public_boundary)
            );
            assert!(e.boundaries.iter().any(|b| b.starts_with("SOURCE_HTTP_")));
        }
    }
    #[test]
    fn direct_mapping_wins_over_inherited_mapping() {
        let direct = CONTROLLER.replace(
            "override fun",
            "@org.springframework.web.bind.annotation.PostMapping(\"/own\") override fun",
        );
        let e = evidence(&[("DealApi.kt", API), ("DealController.kt", &direct)]);
        let public: Vec<_> = e
            .entrypoints
            .iter()
            .filter(|p| super::super::process_candidates::public_boundary(p))
            .collect();
        assert_eq!(public.len(), 1);
        assert_eq!(public[0].trigger["paths"], json!(["/own"]));
        assert!(
            !e.observations
                .values()
                .any(|o| o.kind == "SOURCE_INHERITED_HTTP_MAPPING")
        );
    }
    #[test]
    fn ambiguous_generic_nullable_and_partial_interfaces_do_not_become_starts() {
        let other = API.replace("DealApi", "SecondApi");
        let ambiguous = CONTROLLER
            .replace(
                "import api.DealApi",
                "import api.DealApi\nimport api.SecondApi",
            )
            .replace(": DealApi", ": DealApi, SecondApi");
        let generic = API.replace("interface DealApi", "interface DealApi<T>");
        let nullable = API.replace("source: String", "source: String?");
        let partial = API.replace("String\n}", "String }");
        for files in [
            vec![
                ("DealApi.kt", API),
                ("SecondApi.kt", &other),
                ("DealController.kt", &ambiguous),
            ],
            vec![("DealApi.kt", &generic), ("DealController.kt", CONTROLLER)],
            vec![("DealApi.kt", &nullable), ("DealController.kt", CONTROLLER)],
            vec![("DealApi.kt", &partial), ("DealController.kt", CONTROLLER)],
        ] {
            let e = evidence(&files);
            assert!(
                !e.entrypoints
                    .iter()
                    .any(super::super::process_candidates::public_boundary),
                "{:#?}",
                e.entrypoints
            );
        }
    }
}
