//! Projection of the bounded javac immutable-local profile into portable equations.
//! The retained-index loader remains responsible for authenticating compiler runs.
use crate::canonical;
use crate::error::{ClewError, ErrorCode};
use crate::java_adapter_v2::{JavaCompilerFact, JavaCompilerIndex};
use clew_facts::values::{
    ModelBinding, ObservationAuthority, ValueModel, ValueNode, ValueOperation,
};
use serde::{Deserialize, Serialize};

pub const SCHEMA: &str = "javac-value-projection/0.1";
pub const POLICY: &str = "javac-immutable-local-values/0.1";
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JavaValueProjection {
    pub schema: String,
    pub owner: String,
    pub nodes: Vec<JavaValueNode>,
    pub global_boundaries: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JavaValueNode {
    pub id: String,
    pub operation: ValueOperation,
    pub guards: Vec<String>,
    pub file: String,
    pub start: u64,
    pub end: u64,
    pub byte_start: u64,
    pub byte_end: u64,
    pub start_line: u64,
    pub end_line: u64,
}
fn invalid(message: &'static str) -> ClewError {
    ClewError::new(ErrorCode::StateCorrupt, message)
}
fn hash<T: Serialize>(value: &T) -> Result<String, ClewError> {
    canonical::hash(value).map_err(|_| invalid("Java value binding serialization failed"))
}
fn binding() -> ModelBinding {
    ModelBinding {
        language: "language:java".into(),
        producer: POLICY.into(),
        snapshot_digest: format!("sha256:{}", "0".repeat(64)),
        facts_digest: format!("sha256:{}", "0".repeat(64)),
        configuration_digest: format!("sha256:{}", "0".repeat(64)),
        policy_digest: canonical::hash_bytes(POLICY.as_bytes()),
        observation_authority: ObservationAuthority::CompilerObserved,
    }
}
fn model(projection: &JavaValueProjection, binding: ModelBinding, fact_digest: &str) -> ValueModel {
    ValueModel {
        schema: clew_facts::values::SCHEMA.into(),
        binding,
        nodes: projection
            .nodes
            .iter()
            .map(|n| ValueNode {
                id: n.id.clone(),
                operation: n.operation.clone(),
                guards: n.guards.clone(),
                evidence_ids: vec![format!("{fact_digest}#{}", n.id)],
            })
            .collect(),
        global_boundaries: projection.global_boundaries.clone(),
    }
}

pub(crate) fn validate_projection_fact(fact: &JavaCompilerFact) -> Result<(), ClewError> {
    let JavaCompilerFact::Declaration {
        value_projection: Some(p),
        symbol_identity,
        declaration_kind,
        file,
        start: Some(start),
        end: Some(end),
        byte_start: Some(byte_start),
        byte_end: Some(byte_end),
        start_line: Some(start_line),
        end_line: Some(end_line),
        resolution,
        ..
    } = fact
    else {
        if matches!(
            fact,
            JavaCompilerFact::Declaration {
                value_projection: Some(_),
                ..
            }
        ) {
            return Err(invalid(
                "Java value projection requires exact enclosing source",
            ));
        }
        return Ok(());
    };
    if p.schema != SCHEMA
        || &p.owner != symbol_identity
        || declaration_kind != "METHOD"
        || resolution != "COMPILER_EXACT"
        || p.nodes.is_empty()
        || p.nodes.len() > 128
        || p.global_boundaries != ["JAVA_VALUE_EFFECT_EXCEPTION_MODEL_UNAVAILABLE"]
    {
        return Err(invalid(
            "Java value projection authority or bounds are invalid",
        ));
    }
    for n in &p.nodes {
        if matches!(n.operation, ValueOperation::Join { .. })
            || n.guards.len() > 1
            || n.operation.operands().iter().any(|id| id.len() > 128)
            || n.guards.iter().any(|id| id.len() > 128)
            || n.file.len() > 4096
        {
            return Err(invalid("Java value references exceed profile bounds"));
        }
        if &n.file != file
            || n.start < *start
            || n.end > *end
            || n.start >= n.end
            || n.byte_start < *byte_start
            || n.byte_end > *byte_end
            || n.byte_start >= n.byte_end
            || n.start_line < *start_line
            || n.end_line > *end_line
            || n.start_line > n.end_line
            || (n.id != "result" && n.id != format!("v{}_{}", n.start, n.end))
            || matches!(n.operation, ValueOperation::Join { .. })
        {
            return Err(invalid("Java value node escapes its source or profile"));
        }
        let allowed_guards = match &n.operation {
            ValueOperation::Coalesce { left, .. } => vec![left.clone()],
            _ => vec![],
        };
        if n.guards != allowed_guards {
            return Err(invalid("Java value guards differ from profile"));
        }
        if let ValueOperation::Unknown { boundary } = &n.operation
            && !matches!(
                boundary.as_str(),
                "JAVA_VALUE_BODY_OR_BUDGET_UNSUPPORTED" | "JAVA_VALUE_EXPRESSION_UNSUPPORTED"
            )
        {
            return Err(invalid("Java value transfer boundary is invalid"));
        }
    }
    if canonical::bytes(p)
        .map_err(|_| invalid("Java value serialization failed"))?
        .len()
        > 32 * 1024
    {
        return Err(invalid("Java value projection byte budget exceeded"));
    }
    let Some(root) = p.nodes.iter().find(|n| n.id == "result") else {
        return Err(invalid("Java value result definition is missing"));
    };
    if !matches!(
        root.operation,
        ValueOperation::Copy { .. } | ValueOperation::Unknown { .. }
    ) {
        return Err(invalid("Java value result transfer is invalid"));
    }
    model(p, binding(), "projection-validation")
        .validate()
        .map_err(invalid)
}

/// Build from a validated compiler index. This validates the closed projection
/// and index bindings, not the authenticity of an arbitrary deserialized index.
/// Publication must use an authenticated retained compiler run. Old indexes
/// without the new projection fail explicitly instead of becoming empty models.
pub fn project_index(index: &JavaCompilerIndex, owner: &str) -> Result<ValueModel, ClewError> {
    crate::java_adapter_v2::validate_index(index)?;
    let matches: Vec<_> = index
        .facts
        .iter()
        .filter(|f| {
            matches!(f,
        JavaCompilerFact::Declaration { symbol_identity, .. } if symbol_identity == owner)
        })
        .collect();
    let [fact] = matches.as_slice() else {
        return Err(invalid("Java value owner is absent or ambiguous"));
    };
    let JavaCompilerFact::Declaration {
        value_projection: Some(projection),
        ..
    } = fact
    else {
        return Err(invalid(
            "Java value projection is unavailable in this index",
        ));
    };
    let source_bindings: Vec<_> = index
        .facts
        .iter()
        .filter_map(|f| match f {
            JavaCompilerFact::SourceFile {
                file,
                source_content_digest,
                ..
            } => Some((file, source_content_digest)),
            _ => None,
        })
        .collect();
    let selected_sources: Vec<_> = source_bindings
        .iter()
        .filter(|(file, _)| projection.nodes.iter().all(|n| &n.file == *file))
        .collect();
    if selected_sources.len() != 1
        || selected_sources[0]
            .1
            .strip_prefix("sha256:")
            .is_none_or(|digest| {
                digest.len() != 64
                    || !digest
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            })
    {
        return Err(invalid(
            "Java value source binding is absent, ambiguous or invalid",
        ));
    }
    if !index.model.source_files.contains(&projection.nodes[0].file) {
        return Err(invalid("Java value source is outside the admitted model"));
    }
    if !index.facts.iter().any(|f| {
        matches!(f, JavaCompilerFact::SourceFile { file, resolution, .. }
        if file == &projection.nodes[0].file && resolution == "SOURCE_MEMBERSHIP_EXACT")
    }) {
        return Err(invalid("Java value source membership is not exact"));
    }
    let mut b = binding();
    b.snapshot_digest = hash(&source_bindings)?;
    b.facts_digest = hash(index)?;
    b.configuration_digest = hash(&index.model)?;
    let mut result = model(projection, b, &hash(fact)?);
    for f in &index.facts {
        if let JavaCompilerFact::Boundary { code, .. } = f {
            result.global_boundaries.push(code.clone());
        }
    }
    result.global_boundaries.sort();
    result.global_boundaries.dedup();
    result.validate().map_err(invalid)?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::java_adapter_v2::{JAVA_ANALYZER_SOURCE, JAVA_FACT_SCHEMA, JAVA_INDEX_SCHEMA};
    use crate::java_project_model::{JAVA_MODEL_SCHEMA, JavaBuildSystem, JavaProjectModel};
    use crate::thread_values::{Nullness, ValueQuery, analyze};
    use std::{collections::BTreeMap, fs, process::Command};

    fn fixture_model() -> JavaProjectModel {
        let mut m = JavaProjectModel {
            schema: JAVA_MODEL_SCHEMA.into(),
            model_digest: String::new(),
            build_system: JavaBuildSystem::Gradle,
            compilation: ":/main".into(),
            source_files: vec!["Values.java".into()],
            classpath: vec![],
            dependency_sources: vec![],
            release: 17,
            compiler_version: "javac 17".into(),
            compiler_options: vec![],
            annotation_processors: vec![],
            annotation_processor_paths: vec![],
            boundaries: vec![],
        };
        m.model_digest = hash(&m).unwrap();
        m
    }
    fn index(mut facts: Vec<JavaCompilerFact>) -> JavaCompilerIndex {
        facts.sort_by_cached_key(|f| canonical::bytes(f).unwrap());
        JavaCompilerIndex {
            schema: JAVA_INDEX_SCHEMA.into(),
            compilation: ":/main".into(),
            model: fixture_model(),
            analyzer_digest: canonical::hash_bytes(JAVA_ANALYZER_SOURCE.as_bytes()),
            facts,
            provenance: None,
            source_state: None,
        }
    }
    fn synthetic_fact() -> JavaCompilerFact {
        serde_json::from_value(serde_json::json!({
            "kind":"DECLARATION","schema":JAVA_FACT_SCHEMA,"declarationKind":"METHOD",
            "name":"literal","symbolIdentity":"method:class:Values#literal()Ljava/lang/String;",
            "ownerIdentity":"class:Values","modifiers":[],"annotations":[],"file":"Values.java",
            "start":0,"end":100,"byteStart":0,"byteEnd":100,"startLine":1,"endLine":1,"resolution":"COMPILER_EXACT",
            "valueProjection":{"schema":SCHEMA,"owner":"method:class:Values#literal()Ljava/lang/String;",
                "nodes":[{"id":"result","operation":{"kind":"UNKNOWN","boundary":"JAVA_VALUE_BODY_OR_BUDGET_UNSUPPORTED"},
                    "guards":[],"file":"Values.java","start":0,"end":100,"byteStart":0,"byteEnd":100,"startLine":1,"endLine":1}],
                "globalBoundaries":["JAVA_VALUE_EFFECT_EXCEPTION_MODEL_UNAVAILABLE"]}
        })).unwrap()
    }
    #[test]
    fn projection_rejects_foreign_owner_clipped_spans_missing_roots_and_open_operations() {
        let fact = synthetic_fact();
        assert!(validate_projection_fact(&fact).is_ok());
        for (key, value) in [
            ("owner", serde_json::json!("method:class:Other#f()V")),
            ("schema", serde_json::json!("latest")),
        ] {
            let mut row = serde_json::to_value(&fact).unwrap();
            row["valueProjection"][key] = value;
            assert!(validate_projection_fact(&serde_json::from_value(row).unwrap()).is_err());
        }
        for (key, value) in [
            ("file", serde_json::json!("Other.java")),
            ("byteEnd", serde_json::json!(101)),
            ("id", serde_json::json!("v0_100")),
            ("guards", serde_json::json!(["result"])),
            (
                "operation",
                serde_json::json!({"kind":"JOIN","values":["result"]}),
            ),
        ] {
            let mut row = serde_json::to_value(&fact).unwrap();
            row["valueProjection"]["nodes"][0][key] = value;
            assert!(
                validate_projection_fact(&serde_json::from_value(row).unwrap()).is_err(),
                "{key}"
            );
        }
    }
    #[test]
    fn old_projection_and_missing_source_membership_fail_explicitly() {
        let fact = synthetic_fact();
        let owner = "method:class:Values#literal()Ljava/lang/String;";
        assert!(project_index(&index(vec![fact.clone()]), owner).is_err());
        let mut row = serde_json::to_value(&fact).unwrap();
        row.as_object_mut().unwrap().remove("valueProjection");
        assert!(project_index(&index(vec![serde_json::from_value(row).unwrap()]), owner).is_err());
        let mut stale = index(vec![fact]);
        stale.analyzer_digest = canonical::hash_bytes(b"old-analyzer");
        assert!(project_index(&stale, owner).is_err());
    }
    #[test]
    fn projection_preserves_parent_boundaries_and_invalidates_on_source_change() {
        let owner = "method:class:Values#literal()Ljava/lang/String;";
        let source = |bytes: &[u8]| JavaCompilerFact::SourceFile {
            schema: JAVA_FACT_SCHEMA.into(),
            file: "Values.java".into(),
            source_content_digest: canonical::hash_bytes(bytes),
            resolution: "SOURCE_MEMBERSHIP_EXACT".into(),
        };
        let mut i = index(vec![synthetic_fact(), source(b"first")]);
        let first = project_index(&i, owner).unwrap();
        i.facts = index(vec![synthetic_fact(), source(b"changed")]).facts;
        let mut foreign = i.clone();
        foreign.model.source_files = vec!["Other.java".into()];
        foreign.model.model_digest.clear();
        foreign.model.model_digest = hash(&foreign.model).unwrap();
        assert!(project_index(&foreign, owner).is_err());
        let mut ambiguous = i.clone();
        ambiguous.facts.push(source(b"second binding"));
        ambiguous
            .facts
            .sort_by_cached_key(|f| canonical::bytes(f).unwrap());
        assert!(project_index(&ambiguous, owner).is_err());
        let changed = project_index(&i, owner).unwrap();
        assert_ne!(
            first.binding.snapshot_digest,
            changed.binding.snapshot_digest
        );
        i.facts.push(JavaCompilerFact::Boundary {
            schema: JAVA_FACT_SCHEMA.into(),
            code: "TEST_PARTIAL_SOURCE".into(),
            diagnostic_code: None,
            line: None,
            file: None,
            start: None,
            end: None,
            start_line: None,
            end_line: None,
            byte_start: None,
            byte_end: None,
            required_checks: vec![],
            resolution: "SOURCE_SCOPE_LIMIT".into(),
        });
        i.facts.sort_by_cached_key(|f| canonical::bytes(f).unwrap());
        assert!(
            project_index(&i, owner)
                .unwrap()
                .global_boundaries
                .contains(&"TEST_PARTIAL_SOURCE".into())
        );
    }

    #[test]
    #[ignore = "qualification launches the real JDK compiler analyzer"]
    fn native_javac_extracts_values_and_fails_closed_on_writes_control_calls_and_budget() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let large = (0..70)
            .map(|i| format!("String x{i}=\"value\";"))
            .collect::<String>();
        let wide = (0..55)
            .map(|i| format!("String wide{i}=\"value\";"))
            .collect::<String>();
        let deep = format!("{}\"value\"{}", "(".repeat(80), ")".repeat(80));
        let oversized_name = "m".repeat(33_000);
        let file = format!("src/{}/Values.java", "longpath".repeat(20));
        fs::create_dir_all(root.join(&file).parent().unwrap()).unwrap();
        let source = format!(
            r#"class Values {{
            String choose(String status) {{ String selected=status == null ? "PENDING" : status; return selected; }}
            String copy(String input) {{ final String output=input; return output; }}
            String copy(int input) {{ return "number"; }}
            String literal() {{ return "🧵"; }}
            String nullable() {{ return null; }}
            String field="value";
            String readField() {{ return field; }}
            String call() {{ return literal(); }}
            String mutate(String input) {{ String x="yes"; Object y=(x=null); return x; }}
            String branch(String input) {{ if(input==null) return "default"; return input; }}
            String large() {{ {large} return "result"; }}
            String byteWide() {{ {wide} return "result"; }}
            String deep() {{ return {deep}; }}
        }}"#
        );
        fs::write(root.join(&file), &source).unwrap();
        fs::write(root.join("sources"), format!("{file}\n")).unwrap();
        fs::write(root.join("classpath"), "").unwrap();
        fs::create_dir(root.join("empty")).unwrap();
        let analyzer = root.join("CodeclewJavaAnalyzer.java");
        fs::write(&analyzer, JAVA_ANALYZER_SOURCE).unwrap();
        let run = || {
            Command::new("java")
                .arg(&analyzer)
                .arg(root)
                .arg(root.join("sources"))
                .arg(root.join("classpath"))
                .args(["17", "", "", "", "", "CLOSED_NO_AP"])
                .arg(root.join("empty"))
                .output()
                .unwrap()
        };
        let first = run();
        assert!(first.status.success(), "compiler analyzer did not succeed");
        let repeat = run();
        assert!(repeat.status.success());
        assert_eq!(first.stdout, repeat.stdout);
        let facts = crate::java_adapter_v2::parse_java_compiler_output(&first.stdout).unwrap();
        assert!(!facts.iter().any(|f| matches!(
            f,
            JavaCompilerFact::Boundary {
                diagnostic_code: Some(_),
                ..
            }
        )));
        let mut operational = crate::java_project_model::JavaOperationalModel {
            authority: fixture_model(),
            source_paths: vec![root.join(&file)],
            classpath_paths: vec![],
            annotation_processor_paths: vec![],
            java_executable: "java".into(),
        };
        // The qualification fixture admits its exact source and isolated classpath
        // directly. It does not claim a native Gradle/Maven model discovery run.
        let version = Command::new("java").arg("--version").output().unwrap();
        assert!(version.status.success());
        let version = String::from_utf8(version.stdout).unwrap();
        operational.authority.source_files = vec![file.clone()];
        operational.authority.compiler_version =
            format!("javac {}", version.split_whitespace().nth(1).unwrap());
        operational.authority.model_digest.clear();
        operational.authority.model_digest = hash(&operational.authority).unwrap();
        let digests = BTreeMap::from([(file.clone(), canonical::hash_bytes(source.as_bytes()))]);
        let i = crate::java_adapter_v2::project_java_compiler_output(
            &first.stdout,
            &operational,
            &digests,
            false,
            None,
            &[],
        )
        .unwrap();
        for (name, signature, expected) in [
            ("choose", "(Ljava/lang/String;)", Nullness::NonNull),
            ("copy", "(Ljava/lang/String;)", Nullness::Unknown),
            ("copy", "(I)", Nullness::NonNull),
            ("literal", "()", Nullness::NonNull),
            ("nullable", "()", Nullness::Null),
            ("readField", "()", Nullness::Unknown),
            ("call", "()", Nullness::Unknown),
            ("mutate", "(Ljava/lang/String;)", Nullness::Unknown),
            ("branch", "(Ljava/lang/String;)", Nullness::Unknown),
            ("large", "()", Nullness::Unknown),
            ("byteWide", "()", Nullness::Unknown),
            ("deep", "()", Nullness::Unknown),
        ] {
            let owner = format!("method:class:Values#{name}{signature}Ljava/lang/String;");
            let m = project_index(&i, &owner).unwrap();
            let r = analyze(
                &m,
                &ValueQuery {
                    target: "result".into(),
                    max_nodes: 128,
                    max_transfers: 2048,
                },
            )
            .unwrap();
            assert_eq!(
                r.observations
                    .iter()
                    .find(|n| n.id == "result")
                    .unwrap()
                    .normal_value_nullness,
                expected,
                "{name}"
            );
            assert!(!r.program_semantic_closure);
            assert_eq!(r.calculation_authority, "DECLARED_MODEL_EQUATIONS");
            if ["mutate", "branch", "large", "byteWide", "deep"].contains(&name) {
                assert!(
                    r.frontier
                        .iter()
                        .any(|f| f.code == "JAVA_VALUE_BODY_OR_BUDGET_UNSUPPORTED")
                );
            }
            if name == "choose" {
                assert!(m.nodes.iter().any(|n| n.operation == ValueOperation::Input));
            }
        }
        let literal = i
            .facts
            .iter()
            .find_map(|f| match f {
                JavaCompilerFact::Declaration {
                    name: Some(name),
                    value_projection: Some(p),
                    ..
                } if name == "literal" => Some(p),
                _ => None,
            })
            .unwrap();
        assert!(
            literal
                .nodes
                .iter()
                .any(|n| n.byte_end - n.byte_start > n.end - n.start)
        );
        // A giant owner exceeds the existing annotation contract independently
        // of this projection. Check the emitter boundary without weakening that
        // contract or claiming an admitted compiler-index result for this input.
        fs::write(
            root.join(&file),
            format!("class Values {{ String {oversized_name}() {{ return \"value\"; }} }}"),
        )
        .unwrap();
        let giant = run();
        assert!(giant.status.success());
        let rows: Vec<serde_json::Value> = String::from_utf8(giant.stdout.clone())
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert!(
            rows.iter()
                .any(|row| row["code"] == "JAVA_VALUE_PROJECTION_BYTES_UNSUPPORTED")
        );
        let method = rows
            .iter()
            .find(|row| row["declarationKind"] == "METHOD")
            .unwrap();
        assert!(method.get("valueProjection").is_none());
        assert!(crate::java_adapter_v2::parse_java_compiler_output(&giant.stdout).is_err());
    }
}
