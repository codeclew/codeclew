use clew::canonical;
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

fn copy_fixture(source: &Path, target: &Path) {
    for entry in WalkDir::new(source).into_iter().map(Result::unwrap) {
        let relative = entry.path().strip_prefix(source).unwrap();
        if relative.components().any(|part| {
            matches!(
                part.as_os_str().to_str(),
                Some(".gradle" | ".semantic-thread" | "build")
            )
        }) {
            continue;
        }
        let destination = target.join(relative);
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&destination).unwrap();
        } else {
            std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
            std::fs::copy(entry.path(), destination).unwrap();
        }
    }
}

fn copy_worker_distribution(source: &Path, target: &Path) {
    for entry in WalkDir::new(source).into_iter().map(Result::unwrap) {
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
fn indexes_constructor_and_null_coalescing_facts_on_kotlin_24() {
    let root = workspace_root();
    let fixture = root.join("fixtures/kotlin-basic");
    let diagnostic_workspace = tempfile::tempdir().unwrap();
    copy_worker_distribution(
        &root.join("workers/kotlin/build/install/kotlin"),
        &diagnostic_workspace
            .path()
            .join("workers/kotlin/build/install/kotlin"),
    );
    let mut worker = WorkerClient::start(diagnostic_workspace.path()).unwrap();
    let project = worker
        .request(
            RequestKind::OpenProject,
            &json!({"repo":fixture,"compilation":":/main"}),
        )
        .unwrap();
    assert_eq!(project["compilerVersion"], "2.4.10");
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

    assert_eq!(index["k2Validated"], true, "{:#}", index["diagnostics"]);
    for graph in ["declarationDescriptors", "declarationRelations"] {
        assert_eq!(
            index[graph]["provenance"]["extractorSchema"],
            "fir-facts-extractor/0.6"
        );
        assert_eq!(index[graph]["provenance"]["compilerVersion"], "2.4.10");
        assert_eq!(index[graph], repeated[graph]);
    }
    assert_eq!(
        index["declarationDescriptorHash"],
        repeated["declarationDescriptorHash"]
    );
    assert_eq!(
        index["declarationRelationHash"],
        repeated["declarationRelationHash"]
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
    assert!(
        constructor["symbolIdentity"]
            .as_str()
            .is_some_and(
                |identity| identity.starts_with("constructor:") && identity.contains("#jvm:")
            )
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
        assert!(
            null_policy[occurrence]["end"].as_u64().unwrap()
                > null_policy[occurrence]["start"].as_u64().unwrap()
        );
    }
    assert_eq!(
        mapping
            .iter()
            .find(|row| row["parameterIndex"] == 1)
            .unwrap()["argumentStart"],
        null_policy["mergedOccurrence"]["start"]
    );
    assert!(
        null_policy["sourceTarget"]
            .as_str()
            .is_some_and(|target| target.contains("compilerNullableSource"))
    );
    assert!(
        null_policy["fallbackTarget"]
            .as_str()
            .is_some_and(|target| target.contains("compilerFallback") && !target.contains("Decoy"))
    );
    assert!(
        null_policy["cfgNodeIds"]
            .as_array()
            .is_some_and(|ids| !ids.is_empty())
    );
    assert_eq!(null_policy["orderProvenance"], "K2_FIR_CFG");

    assert!(
        index["declarationRelations"]["boundaries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|boundary| {
                boundary["resolution"] == "UNKNOWN"
                    && boundary["stage"] == "NULL_POLICY"
                    && matches!(
                        boundary["code"].as_str(),
                        Some(
                            "SAFE_CALL_POLICY_UNSUPPORTED"
                                | "UNRESOLVED_NULLABLE_SOURCE_OCCURRENCE"
                        )
                    )
            })
    );
    assert!(
        index["declarationDescriptors"]["boundaries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|boundary| {
                boundary["resolution"] == "UNKNOWN"
                    && matches!(
                        boundary["code"].as_str(),
                        Some(
                            "LOCAL_CONSTRUCTOR_UNSUPPORTED"
                                | "GENERATED_OR_NO_SOURCE"
                                | "UNRESOLVED_CONSTRUCTOR_DESCRIPTOR"
                        )
                    )
            })
    );
    assert!(
        relation_rows(&index, "NULL_COALESCES")
            .iter()
            .all(|relation| !relation["fallbackTarget"]
                .as_str()
                .is_some_and(|target| target.contains("Decoy")))
    );
    worker.shutdown().unwrap();
}

#[test]
fn indexes_direct_return_value_relations_on_kotlin_24() {
    let root = workspace_root();
    let fixture = root.join("fixtures/kotlin-basic");
    let diagnostic_workspace = tempfile::tempdir().unwrap();
    copy_worker_distribution(
        &root.join("workers/kotlin/build/install/kotlin"),
        &diagnostic_workspace
            .path()
            .join("workers/kotlin/build/install/kotlin"),
    );
    let mut worker = WorkerClient::start(diagnostic_workspace.path()).unwrap();
    let project = worker
        .request(
            RequestKind::OpenProject,
            &json!({"repo":fixture,"compilation":":/main"}),
        )
        .unwrap();
    assert_eq!(project["compilerVersion"], "2.4.10");
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
    assert_eq!(index["k2Validated"], true, "{:#}", index["diagnostics"]);
    assert_eq!(
        index["declarationRelations"]["provenance"]["extractorSchema"],
        "fir-facts-extractor/0.6"
    );
    assert_eq!(
        index["declarationRelations"]["provenance"]["compilerVersion"],
        "2.4.10"
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
    assert!(returned.len() >= 2);
    assert!(returned.iter().all(|row| {
        let source_start = row["sourceOccurrence"]["start"].as_u64();
        let source_end = row["sourceOccurrence"]["end"].as_u64();
        let return_start = row["returnOccurrence"]["start"].as_u64();
        let return_end = row["returnOccurrence"]["end"].as_u64();
        row["resolution"] == "PROVEN"
            && row["provider"] == "K2_FIR"
            && matches!(
                row["sourceKind"].as_str(),
                Some("PROPERTY_READ" | "FUNCTION_CALL_RESULT")
            )
            && row["valueProvenance"] == "FIR_RETURN_RESULT_IDENTITY"
            && row["sourceOccurrence"] == row["resultOccurrence"]
            && row["cfgProvenance"]["sourceReachesReturn"] == true
            && row["cfgProvenance"]["sourceDominatesReturn"] == true
            && row["cfgProvenance"]["returnNodeKind"] == "JumpNode"
            && row["evaluationCount"] == 1
            && matches!(
                (source_start, source_end, return_start, return_end),
                (Some(ss), Some(se), Some(rs), Some(re)) if rs <= ss && se <= re
            )
            && row["cfgNodeIds"].as_array().is_some_and(|ids| {
                ids.contains(&row["sourceOccurrence"]["cfgNodeId"])
                    && ids.contains(&row["returnOccurrence"]["cfgNodeId"])
            })
    }));
    let property = returned
        .iter()
        .copied()
        .find(|row| {
            row["owner"] == "com/acme/DirectReturnProjection.returnedProperty"
                && row["target"] == "com/acme/DirectReturnProjection.projected"
        })
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
    let source_start = property["sourceOccurrence"]["start"].as_u64().unwrap();
    let source_end = property["sourceOccurrence"]["end"].as_u64().unwrap();
    let return_start = property["returnOccurrence"]["start"].as_u64().unwrap();
    let return_end = property["returnOccurrence"]["end"].as_u64().unwrap();
    assert!(return_start <= source_start && source_end <= return_end);
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
        .find(|row| {
            row["owner"] == "com/acme/directReturnedCall"
                && row["target"] == "com/acme/internalDescriptor"
        })
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
    for (owner, codes) in [
        (
            "aliasedProperty",
            &["LOCAL_GENERATED_OR_UNRESOLVED_RETURN_VALUE"][..],
        ),
        (
            "branchedProperty",
            &["NON_LINEAR_OR_MULTIPLE_RETURN_FLOW"][..],
        ),
        (
            "implicitProperty",
            &[
                "IMPLICIT_RETURN_UNSUPPORTED",
                "IMPLICIT_OR_MISSING_RETURN_SOURCE",
            ][..],
        ),
        (
            "multipleReturnedCalls",
            &["MULTIPLE_OR_AMBIGUOUS_RETURN_VALUE_OCCURRENCES"][..],
        ),
        (
            "safeReturnedProperty",
            &["NON_LINEAR_OR_MULTIPLE_RETURN_FLOW"][..],
        ),
        (
            "elvisReturnedProperty",
            &["NON_LINEAR_OR_MULTIPLE_RETURN_FLOW"][..],
        ),
        (
            "unresolvedSourceReturn",
            &["LOCAL_GENERATED_OR_UNRESOLVED_RETURN_VALUE"][..],
        ),
    ] {
        assert!(
            has_boundary(owner, codes),
            "missing UNKNOWN boundary for rejected contour"
        );
        assert!(returned.iter().all(|relation| {
            !relation["owner"]
                .as_str()
                .is_some_and(|candidate| candidate.contains(owner))
        }));
    }
    worker.shutdown().unwrap();
}

#[test]
fn indexes_compiler_derived_declaration_descriptors_on_kotlin_24() {
    let root = workspace_root();
    let fixture = root.join("fixtures/kotlin-basic");
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
    assert_eq!(graph["provenance"]["compilerVersion"], "2.4.10");
    assert_eq!(
        graph["provenance"]["projectModelHash"],
        project["projectModelHash"]
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
    let rows = descriptor_rows(&index);
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
    let override_relation = relation_rows(&index, "OVERRIDES")
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
                    && boundary["code"] == "NO_COMPILER_CALLABLE_ID"
                    && boundary["provider"] == "K2_FIR"
                    && boundary["module"] == ":"
                    && boundary["sourceSet"] == "main"
                    && boundary["compilerAuthority"] == "fir-facts-extractor/0.4"
            })
    );
    worker.shutdown().unwrap();
}

#[test]
fn indexes_compiler_derived_declaration_relations_on_kotlin_24() {
    let root = workspace_root();
    let fixture = root.join("fixtures/kotlin-basic");
    let mut worker = WorkerClient::start(&root).unwrap();
    let project = worker
        .request(
            RequestKind::OpenProject,
            &json!({"repo":fixture,"compilation":":/main"}),
        )
        .unwrap();
    assert_eq!(project["compilerVersion"], "2.4.10");
    assert_eq!(project["workerCompilerVersion"], "2.4.10");
    assert_eq!(worker.capabilities.compiler_version, "2.4.10");

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

    assert_eq!(index["k2Validated"], true, "{:#}", index["diagnostics"]);
    let graph = &index["declarationRelations"];
    assert_eq!(graph["schema"], "declaration-relation-graph/0.1");
    assert_eq!(graph["compilation"], ":/main");
    let provenance = &graph["provenance"];
    assert_eq!(provenance["provider"], "COMPILER_SEMANTIC_FACTS");
    assert_eq!(provenance["compilerVersion"], "2.4.10");
    assert_eq!(provenance["projectModelHash"], project["projectModelHash"]);
    assert_eq!(provenance["extractorSchema"], "fir-facts-extractor/0.4");
    assert_eq!(provenance["workerCompilerVersion"], "2.4.10");
    assert_eq!(provenance["workerVersion"], "0.1.0");
    assert_eq!(provenance["workerProtocolVersion"], "1.0");
    assert!(
        provenance["pluginArtifactFingerprint"]
            .as_str()
            .is_some_and(|digest| digest.starts_with("sha256:") && digest.len() == 71)
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

    let overrides = relation_rows(&index, "OVERRIDES");
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
        assert!(!relation_rows(&index, kind).is_empty(), "missing {kind}");
    }
    assert!(relation_rows(&index, "CALLS").iter().any(|relation| {
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
    let reordered = relation_rows(&index, "CALLS")
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
    );
    assert!(relation_rows(&index, "CONSTRUCTS").iter().any(|relation| {
        relation["target"]
            .as_str()
            .unwrap_or_default()
            .contains("Envelope")
    }));
    assert!(relation_rows(&index, "WRITES").iter().any(|relation| {
        relation["target"]
            .as_str()
            .unwrap_or_default()
            .contains("RelationState.field")
            && relation["orderKey"].as_i64().is_some()
    }));
    assert!(relation_rows(&index, "INITIALIZES").iter().any(|relation| {
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
    assert!(relation_rows(&index, "CALLS").iter().all(|relation| {
        let target = relation["target"].as_str().unwrap_or_default();
        !target.starts_with("kotlin/reflect/") && !target.starts_with("java/lang/reflect/")
    }));

    let temporary = tempfile::Builder::new()
        .prefix("kotlin-2-4-relations-")
        .tempdir_in(root.join("fixtures"))
        .unwrap();
    copy_fixture(&fixture, temporary.path());
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

#[test]
fn tampered_semantic_cache_recomputes_compiler_relations() {
    let root = workspace_root();
    let source_fixture = root.join("fixtures/kotlin-basic");
    let temporary = tempfile::Builder::new()
        .prefix("kotlin-2-4-cache-integrity-")
        .tempdir_in(root.join("fixtures"))
        .unwrap();
    copy_fixture(&source_fixture, temporary.path());

    let mut worker = WorkerClient::start(&root).unwrap();
    worker
        .request(
            RequestKind::OpenProject,
            &json!({"repo":temporary.path(),"compilation":":/main"}),
        )
        .unwrap();
    let original = worker
        .request(
            RequestKind::IndexFiles,
            &json!({"repo":temporary.path(),"compilation":":/main","syntaxOnly":false}),
        )
        .unwrap();
    assert!(
        relation_rows(&original, "OVERRIDES")
            .iter()
            .any(|relation| {
                relation["target"]
                    .as_str()
                    .unwrap_or_default()
                    .contains("NumericSource.read")
            })
    );
    worker.shutdown().unwrap();

    let cache_root = temporary.path().join(".semantic-thread/cache/k2");
    let cache_file = WalkDir::new(&cache_root)
        .into_iter()
        .map(Result::unwrap)
        .find(|entry| {
            entry.file_type().is_file()
                && entry.path().extension().and_then(|value| value.to_str()) == Some("json")
        })
        .expect("semantic K2 cache file")
        .into_path();
    let mut cached: Value = serde_json::from_slice(&std::fs::read(&cache_file).unwrap()).unwrap();
    assert_eq!(cached["schema"], "semantic-k2-cache/0.4");
    assert_eq!(cached["authority"], "NON_AUTHORITATIVE");
    let metadata_integrity = cached["payloadIntegrity"].clone();
    let forged = cached["facts"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|fact| fact["recordType"] == "DECLARATION_RELATION" && fact["kind"] == "OVERRIDES")
        .expect("compiler override fact");
    forged["target"] = Value::String("com/acme/LexicalDecoy.read".into());
    let forged_payload = json!({
        "valid":cached["valid"],
        "facts":cached["facts"],
        "diagnostics":cached["diagnostics"],
    });
    cached["payloadIntegrity"] = Value::String(canonical::hash(&forged_payload).unwrap());
    let forged_integrity = cached["payloadIntegrity"].clone();
    assert_ne!(forged_integrity, metadata_integrity);
    std::fs::write(&cache_file, serde_json::to_vec(&cached).unwrap()).unwrap();

    let mut worker = WorkerClient::start(&root).unwrap();
    worker
        .request(
            RequestKind::OpenProject,
            &json!({"repo":temporary.path(),"compilation":":/main"}),
        )
        .unwrap();
    let recomputed = worker
        .request(
            RequestKind::IndexFiles,
            &json!({"repo":temporary.path(),"compilation":":/main","syntaxOnly":false}),
        )
        .unwrap();
    let overrides = relation_rows(&recomputed, "OVERRIDES");
    assert!(overrides.iter().any(|relation| {
        relation["target"]
            .as_str()
            .unwrap_or_default()
            .contains("NumericSource.read")
    }));
    assert!(overrides.iter().all(|relation| {
        !relation["target"]
            .as_str()
            .unwrap_or_default()
            .contains("LexicalDecoy")
    }));
    worker.shutdown().unwrap();

    let repaired: Value = serde_json::from_slice(&std::fs::read(cache_file).unwrap()).unwrap();
    let repaired_payload = json!({
        "valid":repaired["valid"],
        "facts":repaired["facts"],
        "diagnostics":repaired["diagnostics"],
    });
    assert_eq!(
        repaired["payloadIntegrity"],
        canonical::hash(&repaired_payload).unwrap(),
    );
    assert_ne!(repaired["payloadIntegrity"], forged_integrity);
    assert!(
        repaired["facts"]
            .as_array()
            .unwrap()
            .iter()
            .all(|fact| { fact["target"].as_str() != Some("com/acme/LexicalDecoy.read") })
    );
}
