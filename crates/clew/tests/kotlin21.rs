mod support;

use clew::index::RepositoryIndex;
use clew::proto::RequestKind;
use clew::worker::{WorkerClient, workspace_root};
use serde_json::{Value, json};
use std::path::Path;
use walkdir::WalkDir;

fn relation_rows<'a>(index: &'a Value, kind: &str) -> Vec<&'a Value> {
    index["declarationRelations"]["relations"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|relation| relation["kind"] == kind)
        .collect()
}

fn descriptor_rows(index: &Value) -> &[Value] {
    index["declarationDescriptors"]["descriptors"]
        .as_array()
        .unwrap()
}

fn copy_relation_fixture(source: &Path, target: &Path) {
    for relative in ["build.gradle.kts", "settings.gradle.kts", "gradlew"] {
        std::fs::copy(source.join(relative), target.join(relative)).unwrap();
    }
    for entry in WalkDir::new(source.join("src")) {
        let entry = entry.unwrap();
        if !entry.file_type().is_file() {
            continue;
        }
        let relative = entry.path().strip_prefix(source).unwrap();
        let destination = target.join(relative);
        std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
        std::fs::copy(entry.path(), destination).unwrap();
    }
    support::seed_build_caches(target);
}

fn copy_worker_distribution(source: &Path, target: &Path) {
    for entry in WalkDir::new(source) {
        let entry = entry.unwrap();
        let relative = entry.path().strip_prefix(source).unwrap();
        let destination = target.join(relative);
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&destination).unwrap();
        } else if entry.file_type().is_file() {
            std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
            std::fs::copy(entry.path(), &destination).unwrap();
            std::fs::set_permissions(&destination, entry.metadata().unwrap().permissions())
                .unwrap();
        } else {
            panic!(
                "worker distribution contains unsupported path: {}",
                entry.path().display()
            );
        }
    }
}

#[test]
fn indexes_constructor_and_null_coalescing_facts_on_kotlin_21() {
    let root = workspace_root();
    let fixture = root.join("fixtures/kotlin-2-1");
    support::seed_build_caches(&fixture);
    let diagnostic_workspace = tempfile::tempdir().unwrap();
    copy_worker_distribution(
        &root.join("workers/kotlin/build/install/kotlin"),
        &diagnostic_workspace
            .path()
            .join("workers/kotlin/build/install/kotlin"),
    );
    copy_worker_distribution(
        &root.join("workers/kotlin21/build/install/kotlin21"),
        &diagnostic_workspace
            .path()
            .join("workers/kotlin21/build/install/kotlin21"),
    );
    let mut worker = WorkerClient::start(diagnostic_workspace.path()).unwrap();
    let project = worker
        .request(
            RequestKind::OpenProject,
            &json!({"repo":fixture,"compilation":":/main"}),
        )
        .unwrap();
    assert_eq!(project["compilerVersion"], "2.1.21");
    let index = worker
        .request(
            RequestKind::IndexFiles,
            &json!({"repo":fixture,"compilation":":/main","syntaxOnly":false}),
        )
        .unwrap();
    let repeated = worker
        .request(
            RequestKind::IndexFiles,
            &json!({"repo":fixture,"compilation":":/main","syntaxOnly":false}),
        )
        .unwrap();
    assert_eq!(index["k2Validated"], true);
    assert_eq!(
        index["declarationDescriptors"]["provenance"]["extractorSchema"],
        "fir-facts-extractor/0.6"
    );
    assert_eq!(
        index["declarationRelations"]["provenance"]["extractorSchema"],
        "fir-facts-extractor/0.6"
    );
    assert_eq!(
        index["declarationDescriptorHash"],
        repeated["declarationDescriptorHash"]
    );
    assert_eq!(
        index["declarationRelationHash"],
        repeated["declarationRelationHash"]
    );
    assert_eq!(
        index["declarationDescriptors"],
        repeated["declarationDescriptors"]
    );
    assert_eq!(
        index["declarationRelations"],
        repeated["declarationRelations"]
    );

    let descriptors = descriptor_rows(&index);
    let constructor = descriptors
        .iter()
        .find(|descriptor| {
            descriptor["declarationKind"] == "CONSTRUCTOR"
                && descriptor["compilerClassId"] == "com/acme/NullableConstruction"
        })
        .expect("compiler constructor descriptor");
    assert_eq!(constructor["resolution"], "PROVEN");
    assert_eq!(constructor["provider"], "K2_FIR");
    assert_eq!(constructor["compilerAuthority"], "fir-facts-extractor/0.6");
    assert_eq!(
        constructor["ownerIdentity"],
        "class:com/acme/NullableConstruction"
    );
    assert!(constructor["containment"].as_array().is_some_and(|owners| {
        owners
            .iter()
            .any(|owner| owner == "class:com/acme/NullableConstruction")
    }));
    assert!(
        constructor["symbolIdentity"]
            .as_str()
            .is_some_and(
                |identity| identity.starts_with("constructor:") && identity.contains("#jvm:")
            )
    );
    assert!(
        constructor["jvmDescriptor"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
    );
    let parameters = constructor["parameterTypes"].as_array().unwrap();
    assert_eq!(parameters.len(), 2);
    assert!(parameters.iter().enumerate().all(|(index, parameter)| {
        parameter["index"] == index
            && parameter["type"] == "kotlin/String"
            && parameter["nullable"] == false
    }));

    let construction = relation_rows(&index, "CONSTRUCTS")
        .into_iter()
        .find(|relation| {
            relation["owner"]
                .as_str()
                .is_some_and(|owner| owner.contains("constructWithNullPolicy"))
                && relation["target"] == constructor["compilerCallableId"]
        })
        .expect("exact constructor occurrence");
    let mapping = construction["argumentToParameter"].as_array().unwrap();
    assert_eq!(
        mapping
            .iter()
            .map(|row| row["parameterIndex"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        vec![1, 0]
    );
    assert!(mapping.iter().all(|row| {
        let index = row["parameterIndex"].as_u64().unwrap() as usize;
        row["parameterType"] == parameters[index]["type"]
    }));

    let null_policy = relation_rows(&index, "NULL_COALESCES")
        .into_iter()
        .find(|relation| {
            relation["owner"]
                .as_str()
                .is_some_and(|owner| owner.contains("constructWithNullPolicy"))
        })
        .expect("exact compiler null-coalescing relation");
    assert_eq!(null_policy["resolution"], "PROVEN");
    assert_eq!(null_policy["provider"], "K2_FIR");
    assert_eq!(null_policy["sourceOccurrence"]["type"], "kotlin/String?");
    assert_eq!(null_policy["sourceOccurrence"]["nullable"], true);
    assert_eq!(null_policy["fallbackOccurrence"]["type"], "kotlin/String");
    assert_eq!(null_policy["fallbackOccurrence"]["nullable"], false);
    assert_eq!(null_policy["mergedOccurrence"]["type"], "kotlin/String");
    assert_eq!(null_policy["mergedOccurrence"]["nullable"], false);
    assert_eq!(
        null_policy["branchProvenance"]["kind"],
        "FIR_ELVIS_EXPRESSION"
    );
    for occurrence in ["sourceOccurrence", "fallbackOccurrence", "mergedOccurrence"] {
        assert!(null_policy[occurrence]["start"].as_u64().is_some());
        assert!(null_policy[occurrence]["end"].as_u64().is_some());
        assert!(
            null_policy[occurrence]["end"].as_u64().unwrap()
                > null_policy[occurrence]["start"].as_u64().unwrap()
        );
    }
    assert_eq!(
        construction["argumentToParameter"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["parameterIndex"] == 1)
            .unwrap()["argumentStart"],
        null_policy["mergedOccurrence"]["start"]
    );
    assert!(
        null_policy["fallbackTarget"]
            .as_str()
            .is_some_and(|target| target.contains("compilerFallback") && !target.contains("Decoy"))
    );
    assert!(
        null_policy["sourceTarget"]
            .as_str()
            .is_some_and(|target| target.contains("compilerNullableSource"))
    );
    assert!(
        null_policy["cfgNodeIds"]
            .as_array()
            .is_some_and(|ids| !ids.is_empty())
    );
    assert_eq!(null_policy["orderProvenance"], "K2_FIR_CFG");

    let relation_boundaries = index["declarationRelations"]["boundaries"]
        .as_array()
        .unwrap();
    assert!(relation_boundaries.iter().any(|boundary| {
        boundary["resolution"] == "UNKNOWN"
            && boundary["stage"] == "NULL_POLICY"
            && matches!(
                boundary["code"].as_str(),
                Some("SAFE_CALL_POLICY_UNSUPPORTED" | "UNRESOLVED_NULLABLE_SOURCE_OCCURRENCE")
            )
    }));
    let descriptor_boundaries = index["declarationDescriptors"]["boundaries"]
        .as_array()
        .unwrap();
    assert!(descriptor_boundaries.iter().any(|boundary| {
        boundary["resolution"] == "UNKNOWN"
            && matches!(
                boundary["code"].as_str(),
                Some(
                    "LOCAL_CONSTRUCTOR_UNSUPPORTED"
                        | "GENERATED_OR_NO_SOURCE"
                        | "UNRESOLVED_CONSTRUCTOR_DESCRIPTOR"
                )
            )
    }));
    assert!(
        relation_rows(&index, "NULL_COALESCES")
            .iter()
            .all(|relation| {
                !relation["fallbackTarget"]
                    .as_str()
                    .is_some_and(|target| target.contains("Decoy"))
            })
    );
    worker.shutdown().unwrap();
}

#[test]
fn indexes_direct_return_value_relations_on_kotlin_21() {
    let root = workspace_root();
    let fixture = root.join("fixtures/kotlin-2-1");
    support::seed_build_caches(&fixture);
    let diagnostic_workspace = tempfile::tempdir().unwrap();
    copy_worker_distribution(
        &root.join("workers/kotlin/build/install/kotlin"),
        &diagnostic_workspace
            .path()
            .join("workers/kotlin/build/install/kotlin"),
    );
    copy_worker_distribution(
        &root.join("workers/kotlin21/build/install/kotlin21"),
        &diagnostic_workspace
            .path()
            .join("workers/kotlin21/build/install/kotlin21"),
    );
    let mut worker = WorkerClient::start(diagnostic_workspace.path()).unwrap();
    let index = worker
        .request(
            RequestKind::IndexFiles,
            &json!({"repo":fixture,"compilation":":/main","syntaxOnly":false}),
        )
        .unwrap();
    let repeated = worker
        .request(
            RequestKind::IndexFiles,
            &json!({"repo":fixture,"compilation":":/main","syntaxOnly":false}),
        )
        .unwrap();
    assert_eq!(index["k2Validated"], true);
    assert_eq!(
        index["declarationRelations"]["provenance"]["extractorSchema"],
        "fir-facts-extractor/0.6"
    );
    assert_eq!(
        index["declarationRelationHash"],
        repeated["declarationRelationHash"]
    );
    assert_eq!(
        index["declarationRelations"],
        repeated["declarationRelations"]
    );

    let returned = relation_rows(&index, "RETURNS_VALUE_FROM");
    assert_eq!(
        returned.len(),
        2,
        "only direct compiler-resolved returns are proven"
    );
    let property = returned
        .iter()
        .copied()
        .find(|row| row["sourceKind"] == "PROPERTY_READ")
        .expect("direct returned property read");
    assert_eq!(
        property["owner"],
        "com/acme/DirectReturnProjection.returnedProperty"
    );
    assert_eq!(
        property["target"],
        "com/acme/DirectReturnProjection.projected"
    );
    assert_eq!(property["resultType"], "kotlin/String");
    assert_eq!(property["resultNullable"], false);
    assert_eq!(property["evaluationCount"], 1);
    assert_eq!(property["valueProvenance"], "FIR_RETURN_RESULT_IDENTITY");
    assert_eq!(property["cfgProvenance"]["sourceReachesReturn"], true);
    assert_eq!(property["cfgProvenance"]["sourceDominatesReturn"], true);
    assert_eq!(
        property["cfgProvenance"]["sourceNodeKind"],
        "QualifiedAccessNode"
    );
    assert_eq!(property["cfgProvenance"]["returnNodeKind"], "JumpNode");
    assert_eq!(property["orderProvenance"], "K2_FIR_CFG");
    assert_eq!(property["resolution"], "PROVEN");
    assert_eq!(property["provider"], "K2_FIR");
    let property_source_start = property["sourceOccurrence"]["start"].as_u64().unwrap();
    let property_source_end = property["sourceOccurrence"]["end"].as_u64().unwrap();
    let property_return_start = property["returnOccurrence"]["start"].as_u64().unwrap();
    let property_return_end = property["returnOccurrence"]["end"].as_u64().unwrap();
    assert!(property_return_start <= property_source_start);
    assert!(property_source_end <= property_return_end);
    assert_eq!(property["sourceOccurrence"], property["resultOccurrence"]);
    let cfg_ids = property["cfgNodeIds"].as_array().unwrap();
    assert!(cfg_ids.contains(&property["sourceOccurrence"]["cfgNodeId"]));
    assert!(cfg_ids.contains(&property["returnOccurrence"]["cfgNodeId"]));
    assert!(
        !property["target"]
            .as_str()
            .unwrap()
            .contains("sameTypedDecoy")
    );

    let call = returned
        .iter()
        .copied()
        .find(|row| row["sourceKind"] == "FUNCTION_CALL_RESULT")
        .expect("direct returned function call");
    assert_eq!(call["owner"], "com/acme/directReturnedCall");
    assert_eq!(call["target"], "com/acme/internalDescriptor");
    assert_eq!(call["resultType"], "kotlin/String");
    assert_eq!(call["resultNullable"], false);
    assert_eq!(call["evaluationCount"], 1);
    assert_eq!(call["cfgProvenance"]["sourceReachesReturn"], true);
    assert_eq!(call["cfgProvenance"]["sourceDominatesReturn"], true);
    assert_eq!(
        call["cfgProvenance"]["sourceNodeKind"],
        "FunctionCallExitNode"
    );
    assert_eq!(call["cfgProvenance"]["returnNodeKind"], "JumpNode");

    let boundaries = index["declarationRelations"]["boundaries"]
        .as_array()
        .unwrap();
    let has_boundary = |owner_fragment: &str, codes: &[&str]| {
        boundaries.iter().any(|boundary| {
            boundary["resolution"] == "UNKNOWN"
                && boundary["stage"] == "RETURN_VALUE"
                && boundary["owner"]
                    .as_str()
                    .is_some_and(|owner| owner.contains(owner_fragment))
                && boundary["code"]
                    .as_str()
                    .is_some_and(|code| codes.contains(&code))
        })
    };
    assert!(has_boundary(
        "aliasedProperty",
        &["LOCAL_GENERATED_OR_UNRESOLVED_RETURN_VALUE"]
    ));
    assert!(has_boundary(
        "branchedProperty",
        &["NON_LINEAR_OR_MULTIPLE_RETURN_FLOW"]
    ));
    assert!(has_boundary(
        "implicitProperty",
        &[
            "IMPLICIT_RETURN_UNSUPPORTED",
            "IMPLICIT_OR_MISSING_RETURN_SOURCE"
        ]
    ));
    let multiple_return_diagnostics = boundaries
        .iter()
        .filter(|boundary| {
            boundary["stage"] == "RETURN_VALUE"
                && boundary["owner"]
                    .as_str()
                    .is_some_and(|owner| owner.contains("multipleReturnedCalls"))
        })
        .map(|boundary| json!({
            "stage": boundary["stage"],
            "code": boundary["code"],
            "ownerIdentityHash": boundary["ownerIdentityHash"],
            "rootFirKindHash": boundary["rootFirKindHash"],
            "nestedResolvedOccurrenceCount": boundary["nestedResolvedOccurrenceCount"],
            "nestedResolvedOccurrenceKindHashes": boundary["nestedResolvedOccurrenceKindHashes"],
        }))
        .collect::<Vec<_>>();
    eprintln!(
        "safe multiple-return diagnostics: {}",
        serde_json::to_string(&multiple_return_diagnostics).unwrap()
    );
    assert!(has_boundary(
        "multipleReturnedCalls",
        &["MULTIPLE_OR_AMBIGUOUS_RETURN_VALUE_OCCURRENCES"]
    ));
    assert!(has_boundary(
        "safeReturnedProperty",
        &["NON_LINEAR_OR_MULTIPLE_RETURN_FLOW"]
    ));
    assert!(has_boundary(
        "elvisReturnedProperty",
        &["NON_LINEAR_OR_MULTIPLE_RETURN_FLOW"]
    ));
    assert!(has_boundary(
        "unresolvedSourceReturn",
        &["LOCAL_GENERATED_OR_UNRESOLVED_RETURN_VALUE"]
    ));
    for rejected_owner in [
        "aliasedProperty",
        "branchedProperty",
        "implicitProperty",
        "multipleReturnedCalls",
        "safeReturnedProperty",
        "elvisReturnedProperty",
        "unresolvedSourceReturn",
    ] {
        assert!(returned.iter().all(|relation| {
            !relation["owner"]
                .as_str()
                .is_some_and(|owner| owner.contains(rejected_owner))
        }));
    }
    worker.shutdown().unwrap();
}

#[test]
fn selects_matching_kotlin_21_worker_and_resolves_extension_names() {
    let root = workspace_root();
    let fixture = root.join("fixtures/kotlin-2-1");
    support::seed_build_caches(&fixture);
    let mut worker = WorkerClient::start(&root).unwrap();

    let project = worker
        .request(
            RequestKind::OpenProject,
            &json!({"repo":fixture,"compilation":":/main"}),
        )
        .unwrap();
    assert_eq!(project["compilerVersion"], "2.1.21");
    assert_eq!(project["workerCompilerVersion"], "2.1.21");
    assert_eq!(project["languageVersion"], "2.1");
    assert_eq!(project["apiVersion"], "2.1");
    assert_eq!(worker.capabilities.compiler_version, "2.1.21");

    let index = worker
        .request(
            RequestKind::IndexFiles,
            &json!({"repo":fixture,"compilation":":/main"}),
        )
        .unwrap();
    assert_eq!(index["compilerVersion"], "2.1.21");
    assert_eq!(index["k2Validated"], true);
    assert!(
        index["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .all(|diagnostic| {
                !diagnostic["message"]
                    .as_str()
                    .unwrap_or_default()
                    .contains("IncompatibleClassChangeError")
            })
    );

    for symbol in ["applyAdaptive", "com.acme.applyAdaptive"] {
        let resolved = worker
            .request(
                RequestKind::ResolveSymbol,
                &json!({"repo":fixture,"compilation":":/main","symbol":symbol}),
            )
            .unwrap();
        assert_eq!(resolved["declaration"]["name"], "applyAdaptive");
        assert_eq!(resolved["k2Validated"], true);
    }

    let kotlin24 = root.join("fixtures/kotlin-basic");
    let project24 = worker
        .request(
            RequestKind::OpenProject,
            &json!({"repo":kotlin24,"compilation":":/main"}),
        )
        .unwrap();
    assert_eq!(project24["compilerVersion"], "2.4.10");
    assert_eq!(project24["workerCompilerVersion"], "2.4.10");
    assert_eq!(worker.capabilities.compiler_version, "2.4.10");

    worker.shutdown().unwrap();
}

#[test]
fn indexes_compiler_derived_declaration_descriptors_on_kotlin_21() {
    let root = workspace_root();
    let fixture = root.join("fixtures/kotlin-2-1");
    support::seed_build_caches(&fixture);
    let mut worker = WorkerClient::start(&root).unwrap();
    let project = worker
        .request(
            RequestKind::OpenProject,
            &json!({"repo":fixture,"compilation":":/main"}),
        )
        .unwrap();
    let verified = worker
        .index_files_verified(&json!({"repo":fixture,"compilation":":/main","syntaxOnly":false}))
        .unwrap();
    let index = worker.inspect_verified_index(&verified).unwrap();
    let mut repository_index = RepositoryIndex::open_compilation(&fixture, Some(":/main")).unwrap();
    repository_index
        .update_verified(&verified, &worker)
        .unwrap();
    assert!(
        repository_index
            .declaration_descriptors()
            .unwrap()
            .is_some()
    );
    let repeated_verified = worker
        .index_files_verified(&json!({"repo":fixture,"compilation":":/main","syntaxOnly":false}))
        .unwrap();
    let repeated = worker.inspect_verified_index(&repeated_verified).unwrap();

    assert_eq!(index["k2Validated"], true);
    let graph = &index["declarationDescriptors"];
    assert_eq!(graph["schema"], "declaration-descriptor-graph/0.1");
    assert_eq!(graph["compilation"], ":/main");
    assert_eq!(graph["provenance"]["provider"], "COMPILER_SEMANTIC_FACTS");
    assert_eq!(graph["provenance"]["compilerVersion"], "2.1.21");
    assert_eq!(
        graph["provenance"]["projectModelHash"],
        project["projectModelHash"]
    );
    assert_eq!(
        graph["provenance"]["extractorSchema"],
        "fir-facts-extractor/0.4"
    );
    assert_eq!(
        graph["provenance"]["extractorSchema"],
        "fir-facts-extractor/0.4"
    );
    assert!(
        graph["provenance"]["pluginArtifactFingerprint"]
            .as_str()
            .is_some_and(|digest| digest.starts_with("sha256:"))
    );
    assert_eq!(
        index["declarationDescriptorHash"],
        repeated["declarationDescriptorHash"]
    );
    assert_eq!(graph, &repeated["declarationDescriptors"]);
    let rows = descriptor_rows(index);
    assert!(!rows.is_empty());
    let canonical_rows = rows.iter().map(Value::to_string).collect::<Vec<_>>();
    assert!(canonical_rows.windows(2).all(|pair| pair[0] <= pair[1]));
    assert!(rows.iter().all(|descriptor| {
        descriptor["resolution"] == "PROVEN"
            && descriptor["provider"] == "K2_FIR"
            && descriptor["module"] == ":"
            && descriptor["sourceSet"] == "main"
            && descriptor["sourceProvenance"] == "COMPILER_SOURCE_RANGE"
            && descriptor["compilerAuthority"] == "fir-facts-extractor/0.4"
            && descriptor["symbolIdentity"]
                .as_str()
                .is_some_and(|value| !value.is_empty())
            && descriptor["declarationKind"]
                .as_str()
                .is_some_and(|value| !value.is_empty())
            && descriptor["ownerIdentity"]
                .as_str()
                .is_some_and(|value| !value.is_empty())
            && descriptor["containment"].is_array()
            && descriptor["effectiveVisibility"]
                .as_str()
                .is_some_and(|value| !value.is_empty())
            && descriptor["modality"]
                .as_str()
                .is_some_and(|value| !value.is_empty())
            && descriptor.get("sourceText").is_none()
    }));
    let by_callable = |needle: &str| {
        rows.iter()
            .filter(|descriptor| {
                descriptor["compilerCallableId"]
                    .as_str()
                    .is_some_and(|identity| identity.contains(needle))
            })
            .collect::<Vec<_>>()
    };
    assert!(by_callable("publicDescriptor").iter().any(|descriptor| {
        descriptor["visibility"] == "public"
            && descriptor["exportBoundary"] == "PUBLIC_API"
            && descriptor["declarationKind"] == "FUNCTION"
            && descriptor["ownerIdentity"] == "package:com.acme"
            && descriptor["containment"]
                .as_array()
                .is_some_and(Vec::is_empty)
            && descriptor["modality"] == "FINAL"
            && descriptor["symbolIdentity"]
                .as_str()
                .is_some_and(|identity| {
                    identity.starts_with(&format!(
                        "callable:{}#jvm:",
                        descriptor["compilerCallableId"].as_str().unwrap()
                    ))
                })
            && descriptor["parameterTypes"][0]["type"] == "kotlin/String"
            && descriptor["parameterTypes"][0]["nullable"] == false
            && descriptor["returnType"] == "kotlin/String?"
            && descriptor["returnNullable"] == true
    }));
    assert!(by_callable("internalDescriptor").iter().any(|descriptor| {
        descriptor["visibility"] == "internal"
            && descriptor["exportBoundary"] == "MODULE_API"
            && descriptor["parameterTypes"][0]["nullable"] == true
    }));
    assert!(by_callable("privateDescriptor").iter().any(|descriptor| {
        descriptor["visibility"] == "private" && descriptor["exportBoundary"] == "PRIVATE_API"
    }));
    let overloads = by_callable("overloadedDescriptor");
    assert_eq!(overloads.len(), 2);
    assert_ne!(
        overloads[0]["symbolIdentity"],
        overloads[1]["symbolIdentity"]
    );
    assert!(overloads.iter().any(|descriptor| {
        descriptor["parameterTypes"][0]["nullable"] == true && descriptor["returnNullable"] == true
    }));
    assert!(by_callable("genericDescriptor").iter().any(|descriptor| {
        descriptor["typeParameters"]
            .as_array()
            .is_some_and(|items| {
                items.len() == 1
                    && items[0]["index"] == 0
                    && items[0]["compilerName"] == "T"
                    && items[0]["bounds"].as_array().is_some_and(|bounds| {
                        bounds.iter().any(|bound| bound == "kotlin/CharSequence")
                    })
            })
    }));
    assert!(by_callable("IntegerSource.read").iter().any(|descriptor| {
        descriptor["isOverride"] == true
            && descriptor["returnType"] == "kotlin/Int"
            && descriptor["ownerIdentity"] == "class:com/acme/IntegerSource"
            && descriptor["containment"].as_array().is_some_and(|owners| {
                owners
                    .iter()
                    .any(|owner| owner == "class:com/acme/IntegerSource")
            })
    }));
    assert!(by_callable("NumericSource.read").iter().any(|descriptor| {
        descriptor["isOverride"] == false && descriptor["returnType"] == "kotlin/Number"
    }));
    assert!(
        by_callable("LexicalDecoy.read")
            .iter()
            .all(|descriptor| descriptor["isOverride"] == false)
    );
    let override_relation = relation_rows(index, "OVERRIDES")
        .into_iter()
        .find(|relation| {
            relation["owner"]
                .as_str()
                .is_some_and(|owner| owner.contains("IntegerSource.read"))
                && relation["target"]
                    .as_str()
                    .is_some_and(|target| target.contains("NumericSource.read"))
        })
        .unwrap();
    assert!(
        by_callable(override_relation["owner"].as_str().unwrap())
            .iter()
            .any(|descriptor| { descriptor["isOverride"] == true })
    );
    assert!(
        by_callable(override_relation["target"].as_str().unwrap())
            .iter()
            .any(|descriptor| { descriptor["isOverride"] == false })
    );
    assert!(by_callable("RelationState.field").iter().any(|descriptor| {
        descriptor["declarationKind"] == "MUTABLE_PROPERTY"
            && descriptor["declaredType"] == "kotlin/String"
            && descriptor["declaredNullable"] == false
            && descriptor["ownerIdentity"] == "class:com/acme/RelationState"
    }));
    assert!(
        graph["boundaries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|boundary| {
                boundary["resolution"] == "UNKNOWN"
                    && boundary["code"] == "LOCAL_DECLARATION_UNSUPPORTED"
                    && boundary["provider"] == "K2_FIR"
                    && boundary["module"] == ":"
                    && boundary["sourceSet"] == "main"
                    && boundary["compilerAuthority"] == "fir-facts-extractor/0.4"
            })
    );
    worker.shutdown().unwrap();
}

#[test]
fn indexes_compiler_derived_declaration_relations_on_kotlin_21() {
    let root = workspace_root();
    let fixture = root.join("fixtures/kotlin-2-1");
    support::seed_build_caches(&fixture);
    let mut worker = WorkerClient::start(&root).unwrap();
    let project = worker
        .request(
            RequestKind::OpenProject,
            &json!({"repo":fixture,"compilation":":/main"}),
        )
        .unwrap();
    let verified = worker
        .index_files_verified(&json!({"repo":fixture,"compilation":":/main","syntaxOnly":false}))
        .unwrap();
    let index = worker.inspect_verified_index(&verified).unwrap();
    let mut repository_index = RepositoryIndex::open_compilation(&fixture, Some(":/main")).unwrap();
    repository_index
        .update_verified(&verified, &worker)
        .unwrap();
    assert!(repository_index.declaration_relations().unwrap().is_some());
    let repeated_verified = worker
        .index_files_verified(&json!({"repo":fixture,"compilation":":/main","syntaxOnly":false}))
        .unwrap();
    let repeated = worker.inspect_verified_index(&repeated_verified).unwrap();

    assert_eq!(index["k2Validated"], true);
    let graph = &index["declarationRelations"];
    assert_eq!(graph["schema"], "declaration-relation-graph/0.1");
    assert_eq!(graph["compilation"], ":/main");
    assert_eq!(graph["provenance"]["provider"], "COMPILER_SEMANTIC_FACTS");
    assert_eq!(graph["provenance"]["compilerVersion"], "2.1.21");
    assert_eq!(
        graph["provenance"]["projectModelHash"],
        project["projectModelHash"]
    );
    assert_eq!(
        index["declarationRelationHash"],
        repeated["declarationRelationHash"]
    );
    assert_eq!(graph, &repeated["declarationRelations"]);

    let serialized = graph["relations"]
        .as_array()
        .unwrap()
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>();
    assert!(serialized.windows(2).all(|pair| pair[0] <= pair[1]));
    assert!(
        graph["relations"]
            .as_array()
            .unwrap()
            .iter()
            .all(|relation| {
                relation["file"]
                    .as_str()
                    .is_some_and(|file| !file.starts_with('/') && !file.contains(".."))
                    && relation["resolution"] == "PROVEN"
                    && relation["provider"] == "K2_FIR"
                    && relation.get("sourceText").is_none()
            })
    );

    let overrides = relation_rows(index, "OVERRIDES");
    assert!(overrides.iter().any(|relation| {
        relation["owner"]
            .as_str()
            .unwrap_or_default()
            .contains("IntegerSource.read")
            && relation["target"]
                .as_str()
                .unwrap_or_default()
                .contains("NumericSource.read")
            && relation["sourceReturnType"] == "kotlin/Int"
            && relation["baseReturnType"] == "kotlin/Number"
    }));
    assert!(overrides.iter().all(|relation| {
        !relation["target"]
            .as_str()
            .unwrap_or_default()
            .contains("LexicalDecoy")
    }));
    for kind in [
        "CALLS",
        "REFERENCES",
        "CONSTRUCTS",
        "READS",
        "WRITES",
        "INITIALIZES",
    ] {
        assert!(!relation_rows(index, kind).is_empty(), "missing {kind}");
    }
    assert!(relation_rows(index, "CALLS").iter().any(|relation| {
        relation["owner"]
            .as_str()
            .unwrap_or_default()
            .contains("callSource")
            && relation["target"]
                .as_str()
                .unwrap_or_default()
                .contains("NumericSource.read")
            && relation["argumentToParameter"]
                .as_array()
                .is_some_and(|arguments| !arguments.is_empty())
    }));
    let reordered = relation_rows(index, "CALLS")
        .into_iter()
        .find(|relation| {
            relation["owner"]
                .as_str()
                .is_some_and(|owner| owner.contains("callEqualTypesByName"))
                && relation["target"]
                    .as_str()
                    .is_some_and(|target| target.contains("combineEqualTypes"))
        })
        .expect("compiler must emit the reversed named-argument call");
    assert_eq!(
        reordered["argumentToParameter"]
            .as_array()
            .unwrap()
            .iter()
            .map(|argument| argument["parameterIndex"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        vec![1, 0],
        "indices must follow compiler argument occurrence order, not duplicate types or names",
    );
    assert!(relation_rows(index, "CONSTRUCTS").iter().any(|relation| {
        relation["target"]
            .as_str()
            .unwrap_or_default()
            .contains("Envelope")
    }));
    assert!(relation_rows(index, "WRITES").iter().any(|relation| {
        relation["target"]
            .as_str()
            .unwrap_or_default()
            .contains("RelationState.field")
            && relation["orderKey"].as_i64().is_some()
    }));
    assert!(relation_rows(index, "INITIALIZES").iter().any(|relation| {
        relation["target"]
            .as_str()
            .unwrap_or_default()
            .contains("RelationState.initial")
            && relation["orderProvenance"] == "FIR_SOURCE_RANGE"
    }));

    let boundary_codes = graph["boundaries"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|boundary| boundary["code"].as_str())
        .collect::<Vec<_>>();
    assert!(boundary_codes.contains(&"NON_FUNCTION_OVERRIDE_UNSUPPORTED"));
    assert!(boundary_codes.contains(&"DYNAMIC_REFLECTION_BOUNDARY"));
    assert!(boundary_codes.contains(&"EXTERNAL_OR_LOCAL_ARGUMENT_TARGET"));
    assert!(relation_rows(index, "CALLS").iter().all(|relation| {
        let target = relation["target"].as_str().unwrap_or_default();
        !target.starts_with("kotlin/reflect/") && !target.starts_with("java/lang/reflect/")
    }));

    let temporary = tempfile::Builder::new()
        .prefix("kotlin-2-1-relations-")
        .tempdir_in(root.join("fixtures"))
        .unwrap();
    copy_relation_fixture(&fixture, temporary.path());
    let source = temporary
        .path()
        .join("src/main/kotlin/com/acme/RelationFacts.kt");
    let mut content = std::fs::read_to_string(&source).unwrap();
    content.push_str("\nfun unresolvedRelation(): String = missingCompilerTarget()\n");
    std::fs::write(source, content).unwrap();
    worker
        .request(
            RequestKind::OpenProject,
            &json!({"repo":temporary.path(),"compilation":":/main"}),
        )
        .unwrap();
    let unresolved = worker
        .request(
            RequestKind::IndexFiles,
            &json!({"repo":temporary.path(),"compilation":":/main","syntaxOnly":false}),
        )
        .unwrap();
    assert_eq!(unresolved["k2Validated"], false);
    assert!(
        unresolved["declarationRelations"]["boundaries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|boundary| boundary["code"] == "UNRESOLVED_CALLABLE_TARGET")
    );

    worker.shutdown().unwrap();
}
