#[path = "support/java_dependency.rs"]
mod java_dependency;
use clew::java_adapter_v2::JavaCompilerFact;
use java_dependency::{Fixture, OWNER, TARGET};

#[test]
fn constructor_injected_jar_target_retains_exact_binary_origin() {
    let fixture = Fixture::new();
    let index = fixture.index();
    let targets: Vec<_> = index
        .facts
        .iter()
        .filter(|fact| {
            matches!(fact,
                JavaCompilerFact::DependencyTarget { symbol_identity, .. }
                    if symbol_identity == TARGET
            )
        })
        .collect();
    assert_eq!(
        targets.len(),
        1,
        "one shared target despite repeated signature use"
    );
    let target = serde_json::to_value(targets[0]).unwrap();
    assert_eq!(
        target["qualifiedName"],
        "example.integration.InventoryGateway"
    );
    assert_eq!(target["ownerIdentity"], OWNER);
    assert_eq!(target["name"], "reserve");
    assert_eq!(target["jvmDescriptor"], "(Ljava/lang/String;I)Z");
    assert_eq!(
        target["binaryOrigin"]["artifact"]["digest"],
        fixture.model.authority.classpath[0].digest
    );
    assert_eq!(
        target["binaryOrigin"]["classEntry"],
        "example/integration/InventoryGateway.class"
    );
    assert_eq!(
        target["binaryMetadata"]["coordinates"],
        serde_json::json!(["example:inventory:2.3.4"])
    );
    assert_eq!(target["binaryMetadata"]["version"], "2.3.4");
    assert_eq!(target["binaryMetadata"]["versionStatus"], "METADATA_EXACT");
    assert_eq!(target["compilerModule"], "module:unnamed");
    assert_eq!(
        target["binaryMetadata"]["automaticModuleName"],
        "example.inventory"
    );
    assert_eq!(target["sourceStatus"], "SOURCE_NOT_ATTACHED");
    assert_eq!(target["bodyStatus"], "BODY_UNAVAILABLE");
    assert_eq!(target["runtimeImplementationStatus"], "UNRESOLVED");
    assert!(target.get("file").is_none());
    assert!(target.get("documentation").is_none());
    assert!(index.facts.iter().any(|fact| matches!(fact, JavaCompilerFact::Relation { target_identity, resolution, .. } if target_identity == TARGET && resolution == "COMPILER_EXACT")));
    assert!(
        !serde_json::to_string(&index)
            .unwrap()
            .contains(fixture.temporary.path().to_str().unwrap())
    );
}

fn target(index: &clew::java_adapter_v2::JavaCompilerIndex, identity: &str) -> serde_json::Value {
    index
        .facts
        .iter()
        .find_map(|fact| match fact {
            JavaCompilerFact::DependencyTarget {
                symbol_identity, ..
            } if symbol_identity == identity => Some(serde_json::to_value(fact).unwrap()),
            _ => None,
        })
        .unwrap()
}

#[test]
fn external_type_without_method_invocation_retains_binary_origin() {
    let mut fixture = Fixture::new();
    fixture.set_source("package example; import example.integration.InventoryGateway; public class Service { private final InventoryGateway gateway; public Service(InventoryGateway gateway) { this.gateway = gateway; } }");
    let index = fixture.index();
    let type_target = target(&index, OWNER);
    assert_eq!(type_target["declarationKind"], "INTERFACE");
    assert_eq!(
        type_target["jvmDescriptor"],
        "Lexample/integration/InventoryGateway;"
    );
    assert_eq!(
        type_target["binaryOrigin"]["artifact"]["digest"],
        fixture.model.authority.classpath[0].digest
    );
    assert_eq!(type_target["bodyStatus"], "BODY_NOT_APPLICABLE");
    assert!(!index.facts.iter().any(|fact| matches!(fact, JavaCompilerFact::DependencyTarget { symbol_identity, .. } if symbol_identity == TARGET)));
}

#[test]
fn classpath_order_selects_actual_jar_and_metadata() {
    let mut fixture = Fixture::new();
    fixture.prepend_shadow_jar();
    let index = fixture.index();
    let first = target(&index, TARGET);
    assert_eq!(first["binaryOrigin"]["classpathIndex"], 0);
    assert_eq!(
        first["binaryOrigin"]["artifact"]["digest"],
        fixture.model.authority.classpath[0].digest
    );
    assert_eq!(first["binaryMetadata"]["version"], "9.9.9");
    fixture.model.authority.classpath.swap(0, 1);
    fixture.model.classpath_paths.swap(0, 1);
    fixture.rehash_model();
    let second = target(&fixture.index(), TARGET);
    assert_eq!(second["binaryMetadata"]["version"], "2.3.4");
    assert_ne!(
        first["binaryOrigin"]["artifact"]["digest"],
        second["binaryOrigin"]["artifact"]["digest"]
    );
}

#[test]
fn attached_source_is_exact_overload_without_runtime_implementation_claim() {
    let mut fixture = Fixture::new();
    let absent_model_digest = fixture.model.authority.model_digest.clone();
    fixture.attach_sources("package example.integration;\npublic interface InventoryGateway {\n boolean reserve(String sku, int quantity);\n boolean reserve(String sku);\n}\n");
    assert_ne!(absent_model_digest, fixture.model.authority.model_digest);
    let index = fixture.index();
    let attached = target(&index, TARGET);
    assert_eq!(attached["sourceStatus"], "SOURCE_ATTACHED");
    assert_eq!(attached["bodyStatus"], "BODY_UNAVAILABLE");
    assert_eq!(attached["runtimeImplementationStatus"], "UNRESOLVED");
    assert_eq!(
        attached["dependencySource"]["text"],
        "boolean reserve(String sku, int quantity);"
    );
    assert_eq!(
        attached["dependencySource"]["sourceEntry"],
        "example/integration/InventoryGateway.java"
    );
    assert_eq!(
        attached["dependencySource"]["sourceArchive"]["digest"],
        fixture.model.authority.dependency_sources[0]
            .source_archive
            .digest
    );
    assert_eq!(
        attached["dependencySource"]["matchingBasis"],
        "MAVEN_CLASSIFIER_PATH_AND_COMPILER_SIGNATURE"
    );
    let type_target = target(&index, OWNER);
    assert_eq!(type_target["sourceStatus"], "SOURCE_ATTACHED");
    assert_eq!(type_target["bodyStatus"], "BODY_NOT_APPLICABLE");
}

#[test]
fn source_signature_mismatch_does_not_forge_a_source_body() {
    let mut fixture = Fixture::new();
    fixture.attach_sources("package example.integration; public interface InventoryGateway { boolean reserve(String sku); }");
    let attached = target(&fixture.index(), TARGET);
    assert_eq!(attached["sourceStatus"], "SOURCE_SIGNATURE_MISMATCH");
    assert_eq!(attached["bodyStatus"], "BODY_UNAVAILABLE");
    assert!(attached.get("dependencySource").is_none());
}

#[test]
fn changed_source_archive_fails_the_admitted_digest() {
    let mut fixture = Fixture::new();
    let archive = fixture.attach_sources("package example.integration; public interface InventoryGateway { boolean reserve(String sku, int quantity); boolean reserve(String sku); }");
    std::fs::write(archive, b"changed source archive").unwrap();
    let error = clew::java_adapter_v2::build_java_compiler_index(
        fixture.temporary.path(),
        &fixture.model,
        &fixture.source_digests,
        false,
        None,
        &[],
        None,
    )
    .unwrap_err();
    assert_eq!(error.code, clew::error::ErrorCode::InputMutated);
}

#[test]
fn changed_binary_archive_fails_the_admitted_digest() {
    let fixture = Fixture::new();
    std::fs::write(&fixture.model.classpath_paths[0], b"changed binary archive").unwrap();
    let error = clew::java_adapter_v2::build_java_compiler_index(
        fixture.temporary.path(),
        &fixture.model,
        &fixture.source_digests,
        false,
        None,
        &[],
        None,
    )
    .unwrap_err();
    assert_eq!(error.code, clew::error::ErrorCode::InputMutated);
}

#[test]
fn class_directory_signature_has_explicit_source_and_body_boundary() {
    let mut fixture = Fixture::new();
    fixture.use_class_directory();
    let observed = target(&fixture.index(), TARGET);
    assert_eq!(observed["binaryOrigin"]["artifact"]["kind"], "DIRECTORY");
    assert_eq!(
        observed["binaryOrigin"]["artifact"]["digest"],
        fixture.model.authority.classpath[0].digest
    );
    assert_eq!(
        observed["binaryMetadata"]["versionStatus"],
        "METADATA_UNAVAILABLE"
    );
    assert_eq!(observed["sourceStatus"], "SOURCE_NOT_ATTACHED");
    assert_eq!(observed["bodyStatus"], "BODY_UNAVAILABLE");
    assert_eq!(observed["runtimeImplementationStatus"], "UNRESOLVED");
}

#[test]
fn unresolvable_optional_source_preserves_exact_binary_signature() {
    let mut fixture = Fixture::new();
    fixture.attach_sources("package example.integration; public interface InventoryGateway { unknown.Missing reserve(String sku, int quantity); boolean reserve(String sku); }");
    let observed = target(&fixture.index(), TARGET);
    assert_eq!(observed["resolution"], "COMPILER_EXACT");
    assert_eq!(observed["sourceStatus"], "SOURCE_ATTACHMENT_UNVERIFIED");
    assert_eq!(observed["bodyStatus"], "BODY_UNAVAILABLE");
    assert!(observed.get("dependencySource").is_none());
    assert_eq!(observed["sourceBoundary"], "SOURCE_ATTACHMENT_UNVERIFIED");
}

#[test]
fn overlapping_classpath_directories_bind_the_exact_binary_name_path() {
    let mut fixture = Fixture::new();
    let classes = fixture.replace_dependencies(&[(
        "pkg/Target.java",
        "package pkg; public class Target { public int value() { return 7; } }",
    )]);
    let parent = fixture.temporary.path().join("overlap-classes");
    std::fs::create_dir(&parent).unwrap();
    let nested = parent.join("nested");
    std::fs::rename(classes, &nested).unwrap();
    fixture.set_classpath_directories(vec![parent, nested]);
    fixture.set_source("package example; public class Service { private final pkg.Target target; public Service(pkg.Target target) { this.target = target; } public int value() { return target.value(); } }");
    let index = fixture.index();
    for identity in ["class:pkg.Target", "method:class:pkg.Target#value()I"] {
        let observed = target(&index, identity);
        assert_eq!(observed["binaryOrigin"]["classpathIndex"], 1);
        assert_eq!(observed["binaryOrigin"]["classEntry"], "pkg/Target.class");
        assert_eq!(
            observed["binaryOrigin"]["artifact"]["digest"],
            fixture.model.authority.classpath[1].digest
        );
        assert_ne!(
            observed["binaryOrigin"]["artifact"]["digest"],
            fixture.model.authority.classpath[0].digest
        );
        assert_eq!(observed["resolution"], "COMPILER_EXACT");
    }
}

#[test]
fn concrete_attached_body_is_available_without_binary_body_equivalence_claim() {
    let mut fixture = Fixture::new();
    let source = "package example.integration; public class InventoryGateway { public boolean reserve(String sku, int quantity) { return quantity > 0; } public boolean reserve(String sku) { return sku != null; } }";
    fixture.replace_dependencies(&[("example/integration/InventoryGateway.java", source)]);
    // Signature and modifiers match, while the attached body intentionally differs.
    // The contract establishes available source, never bytecode/body equivalence.
    fixture.attach_sources("package example.integration; public class InventoryGateway { public boolean reserve(String sku, int quantity) { return quantity > 10; } public boolean reserve(String sku) { return sku != null; } }");
    let observed = target(&fixture.index(), TARGET);
    assert_eq!(observed["sourceStatus"], "SOURCE_ATTACHED");
    assert_eq!(observed["bodyStatus"], "BODY_SOURCE_AVAILABLE");
    assert_eq!(observed["resolution"], "COMPILER_EXACT");
    assert_eq!(observed["runtimeImplementationStatus"], "UNRESOLVED");
    assert!(
        observed["dependencySource"]["text"]
            .as_str()
            .unwrap()
            .contains("return quantity > 10;")
    );
    assert_eq!(
        observed["dependencySource"]["matchingBasis"],
        "MAVEN_CLASSIFIER_PATH_AND_COMPILER_SIGNATURE"
    );
    assert!(observed.get("bodyEquivalence").is_none());
}

#[test]
fn source_entry_budget_overflow_retains_exact_binary_targets_and_boundary() {
    let mut fixture = Fixture::new();
    let sources: Vec<_> = (0..33)
        .map(|index| {
            (
                format!("example/budget/Target{index:02}.java"),
                format!(
                    "package example.budget; public interface Target{index:02} {{ int value(); }}"
                ),
            )
        })
        .collect();
    let entries: Vec<_> = sources
        .iter()
        .map(|(entry, source)| (entry.as_str(), source.as_str()))
        .collect();
    fixture.replace_dependencies(&entries);
    fixture.attach_source_entries(&entries);
    let fields: String = (0..33)
        .map(|index| format!("private example.budget.Target{index:02} target{index:02}; "))
        .collect();
    let calls = (0..33)
        .map(|index| format!("target{index:02}.value()"))
        .collect::<Vec<_>>()
        .join(" + ");
    fixture.set_source(&format!("package example; public class Service {{ {fields} public int total() {{ return {calls}; }} }}"));
    let index = fixture.index();
    let mut attached = 0;
    let mut overflow = 0;
    for number in 0..33 {
        let observed = target(
            &index,
            &format!("method:class:example.budget.Target{number:02}#value()I"),
        );
        assert_eq!(observed["resolution"], "COMPILER_EXACT");
        assert_eq!(
            observed["binaryOrigin"]["artifact"]["digest"],
            fixture.model.authority.classpath[0].digest
        );
        assert_eq!(observed["runtimeImplementationStatus"], "UNRESOLVED");
        assert_eq!(observed["bodyStatus"], "BODY_UNAVAILABLE");
        match observed["sourceStatus"].as_str().unwrap() {
            "SOURCE_ATTACHED" => {
                attached += 1;
                assert!(observed.get("dependencySource").is_some());
                assert!(observed.get("sourceBoundary").is_none());
            }
            "SOURCE_ATTACHMENT_UNVERIFIED" => {
                overflow += 1;
                assert!(observed.get("dependencySource").is_none());
                assert_eq!(
                    observed["sourceBoundary"],
                    "SOURCE_ATTACHMENT_VERIFICATION_BUDGET_EXCEEDED"
                );
            }
            status => panic!("unexpected source status: {status}"),
        }
    }
    assert_eq!(
        attached, 32,
        "method and type targets share each source-entry verification"
    );
    assert_eq!(overflow, 1);
    assert!(
        !index
            .facts
            .iter()
            .any(|fact| matches!(fact, JavaCompilerFact::Boundary { .. })),
        "optional source budget must not downgrade semantic compiler coverage"
    );
}
