//! Project retained K2 descriptors and compiler-linked PSI flow into documentation.
use super::{digest, invalid};
use crate::error::ClewError;
use serde_json::{Value, json};
use std::collections::BTreeMap;

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
            "language":"kotlin", "profile":"kotlin-jvm-maven-analysis", "compilation":":/main", "targetRef":"main"})).unwrap();
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
                compilation: ":/main".into(),
                target_ref: "main".into(),
                source_link_template: None,
                contract_files: vec![],
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
        let scenario: Scenario = serde_json::from_value(json!({"schema":"codeclew-documentation-scenario/1.0","id":"import","title":"Import stock",
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
    fn eight_kotlin_services_compose_and_ninth_is_an_explicit_boundary() {
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
        let too_many = chain(9);
        assert!(too_many.scenarios["import"].steps.is_empty());
        assert!(
            too_many.scenarios["import"]
                .boundaries
                .contains(&"MORE_THAN_EIGHT_SERVICES_NOT_SUPPORTED_IN_ONE_SCENARIO".into())
        );
    }

    #[test]
    fn explanation_requires_step_evidence_and_supports_eight_services_plus_client() {
        let checked = chain(8);
        let step = &checked.scenarios["import"].steps[0];
        let mut narrative: Narrative = serde_json::from_value(json!({
            "schema":"codeclew-documentation-narrative/1.1", "subject":"scenario:import", "contextDigest":checked.context_digest,
            "operations":[{"id":"import","title":"Import stock","summary":{"id":"summary","text":"Propagate stock changes.","dependencyIds":step.dependency_ids,"sourceIds":step.source_ids},
                "participants":(0..8).map(|i|json!({"id":format!("p{i}"),"label":format!("Service {i}"),"service":format!("service{i}")})).chain([json!({"id":"client","label":"Warehouse operator","service":null})]).collect::<Vec<_>>(),
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
        narrative.operations[0].explanation[0].event_ids = vec!["unknown".into()];
        assert!(render::validate(&narrative, &checked).is_err());
        narrative.operations[0].explanation.clear();
        assert!(render::validate(&narrative, &checked).is_err());
        narrative.schema = "codeclew-documentation-narrative/1.0".into();
        render::validate(&narrative, &checked).unwrap();
    }
}
