//! Retained compiler classpath ownership. Source-to-binary binding remains separate.
use super::{
    digest, invalid,
    model::{Observation, ServiceEvidence},
};
use crate::error::ClewError;
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub(super) fn is_digest(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

fn rows<'a>(fact: &'a Value, field: &str) -> Result<&'a [Value], ClewError> {
    match &fact[field] {
        Value::Null => Ok(&[]),
        Value::Array(rows) if rows.len() <= 8192 => Ok(rows),
        _ => Err(invalid(
            "Kotlin library provenance field is not a bounded array",
        )),
    }
}

fn artifact(value: &Value) -> Result<(), ClewError> {
    let index = value["classpathIndex"]
        .as_u64()
        .filter(|index| *index < 65536);
    let hash = value["contentDigest"]
        .as_str()
        .filter(|hash| is_digest(hash));
    let name = value["fileName"].as_str().filter(|name| {
        !name.is_empty()
            && name.len() <= 512
            && !name.contains(['/', '\\', ':'])
            && !name.chars().any(char::is_control)
    });
    if index.is_none()
        || hash.is_none()
        || name.is_none()
        || value["size"].as_u64().is_none()
        || value["id"]
            != format!(
                "classpath:{}:{}",
                index.unwrap_or_default(),
                hash.unwrap_or_default()
            )
        || value["authority"] != "GRADLE_RESOLVED_COMPONENT_ARTIFACT"
        || !value["coordinates"].is_object()
        || !value["variant"].is_object()
    {
        return Err(invalid(
            "Kotlin resolved library artifact identity is invalid",
        ));
    }
    for key in ["group", "name", "version", "extension", "scope"] {
        if !value["coordinates"][key]
            .as_str()
            .is_some_and(|s| s.len() <= 512 && !s.chars().any(char::is_control))
        {
            return Err(invalid("Kotlin resolved artifact coordinates are invalid"));
        }
    }
    if !value["coordinates"]["classifier"].is_null()
        && !value["coordinates"]["classifier"]
            .as_str()
            .is_some_and(|s| s.len() <= 512)
    {
        return Err(invalid("Kotlin resolved artifact classifier is invalid"));
    }
    Ok(())
}

pub(super) fn project(
    evidence: &mut ServiceEvidence,
    fact: &Value,
    binding: &str,
    scope: &str,
) -> Result<(), ClewError> {
    let mut artifacts = BTreeMap::new();
    for row in rows(fact, "resolvedDependencyArtifacts")? {
        artifact(row)?;
        let index = row["classpathIndex"].as_u64().unwrap();
        if artifacts.insert(index, row).is_some() {
            return Err(invalid("Kotlin resolved artifact repeats a classpath slot"));
        }
        insert(
            evidence,
            "DEPENDENCY_ARTIFACT",
            row["id"].as_str().unwrap(),
            row.clone(),
            binding,
            scope,
        )?;
    }
    for row in rows(fact, "publicationDeclarations")? {
        if !["group", "name", "version"]
            .iter()
            .all(|key| row[key].as_str().is_some_and(|s| s.len() <= 512))
            || !matches!(
                row["authority"].as_str(),
                Some("GRADLE_PUBLICATION_DECLARATION" | "GRADLE_PROJECT_DECLARATION")
            )
        {
            return Err(invalid("Kotlin library publication declaration is invalid"));
        }
        insert(
            evidence,
            "LIBRARY_PUBLICATION",
            &digest(row)?,
            row.clone(),
            binding,
            scope,
        )?;
    }
    for row in rows(fact, "binaryCallTargets")? {
        let target = row["targetIdentity"]
            .as_str()
            .ok_or_else(|| invalid("Kotlin binary target identity is missing"))?;
        crate::semantic_validation::validate_kotlin_full_symbol_identity(target)?;
        let owners = row["owners"]
            .as_array()
            .filter(|rows| rows.len() <= 8192)
            .ok_or_else(|| invalid("Kotlin binary owner evidence is invalid"))?;
        if !owners.is_empty()
            && (row["jvmDescriptor"].as_str()
                != target.split_once("#jvm:").map(|(_, descriptor)| descriptor)
                || !row["ownerInternalName"].as_str().is_some_and(|owner| {
                    !owner.is_empty()
                        && owner.len() <= 1024
                        && owner
                            .split('/')
                            .all(|part| !part.is_empty() && part != "." && part != "..")
                        && owner
                            .chars()
                            .all(|c| c.is_alphanumeric() || "_$/".contains(c))
                }))
        {
            return Err(invalid(
                "Kotlin binary owner and exact descriptor are invalid",
            ));
        }
        for owner in owners {
            if owner["classEntry"]
                != format!(
                    "{}.class",
                    row["ownerInternalName"].as_str().unwrap_or_default()
                )
            {
                return Err(invalid(
                    "Kotlin binary owner class entry differs from compiler-named owner",
                ));
            }
            let index = owner["classpathIndex"]
                .as_u64()
                .ok_or_else(|| invalid("Kotlin binary owner classpath slot is missing"))?;
            if !owner["artifact"].is_null() {
                let captured = artifacts
                    .get(&index)
                    .ok_or_else(|| invalid("Kotlin binary owner names an uncaptured artifact"))?;
                if &owner["artifact"] != *captured {
                    return Err(invalid(
                        "Kotlin binary owner artifact differs from captured classpath identity",
                    ));
                }
            }
        }
        if row["status"] == "UNIQUE_CLASSPATH_OWNER"
            && (owners.len() != 1
                || owners[0]["kind"] != "JAR"
                || owners[0]["methodPresent"] != true
                || owners[0]["artifact"].is_null())
        {
            return Err(invalid(
                "Kotlin binary ownership uniqueness is not supported by its evidence",
            ));
        }
        insert(
            evidence,
            "DEPENDENCY_TARGET",
            target,
            row.clone(),
            binding,
            scope,
        )?;
    }
    Ok(())
}

fn insert(
    e: &mut ServiceEvidence,
    kind: &str,
    key: &str,
    mut row: Value,
    binding: &str,
    scope: &str,
) -> Result<(), ClewError> {
    let id = format!(
        "{}:{}:{}",
        e.service,
        kind.to_ascii_lowercase().replace('_', "-"),
        &digest(&(scope, key))?[7..31]
    );
    row["scope"] = json!(scope);
    row["evidenceBinding"] = json!(binding);
    let observation = Observation {
        id: id.clone(),
        kind: kind.into(),
        service: e.service.clone(),
        symbol: key.into(),
        digest: digest(&row)?,
        normalized: row,
        source_ids: vec![],
    };
    if e.observations.insert(id, observation).is_some() {
        return Err(invalid(
            "Kotlin library provenance repeats an identity in one compilation",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn evidence() -> ServiceEvidence {
        serde_json::from_value(json!({"schema":"evidence","service":"owned","revision":"revision","serviceDigest":"input","extractor":"compiler","runtimeMode":"RELEASE","coverage":"PARTIAL","boundaries":["ORIGINAL_LIMIT"],"entrypoints":[],"observations":{},"sources":{},"contracts":{}})).unwrap()
    }
    fn metadata() -> Value {
        let hash = format!("sha256:{}", "a".repeat(64));
        let artifact = json!({"id":format!("classpath:0:{hash}"),"classpathIndex":0,"fileName":"library.jar","contentDigest":hash,"size":123,"coordinates":{"group":"owned","name":"library","version":"1.0","classifier":null,"extension":"jar","scope":"compileClasspath"},"variant":{"org.gradle.usage":"java-api"},"authority":"GRADLE_RESOLVED_COMPONENT_ARTIFACT","classpathAuthority":"KOTLIN_TASK_LIBRARIES"});
        json!({"resolvedDependencyArtifacts":[artifact.clone()],"publicationDeclarations":[{"group":"owned","name":"library","version":"1.0","authority":"GRADLE_PUBLICATION_DECLARATION"}],"binaryCallTargets":[{"targetIdentity":"callable:owned/library/Policy.decide#jvm:(I)Ljava/lang/String;","ownerInternalName":"owned/library/Policy","jvmDescriptor":"(I)Ljava/lang/String;","methodName":"decide","status":"UNIQUE_CLASSPATH_OWNER","owners":[{"classpathIndex":0,"kind":"JAR","classEntry":"owned/library/Policy.class","methodPresent":true,"artifact":artifact}],"authority":"COMPILER_TARGET_AND_UNIQUE_SELECTED_CLASSPATH_CLASSFILE_NOT_SOURCE_EQUIVALENCE"}]})
    }
    #[test]
    fn library_artifact_projection_keeps_compilations_and_original_limits_separate() {
        let mut e = evidence();
        project(&mut e, &metadata(), "payload", ":a/main").unwrap();
        project(&mut e, &metadata(), "payload", ":b/main").unwrap();
        assert_eq!(e.observations.len(), 6);
        assert_eq!(e.boundaries, ["ORIGINAL_LIMIT"]);
        assert_eq!(e.coverage, "PARTIAL");
        assert_eq!(
            e.observations
                .values()
                .filter(|o| o.normalized["scope"] == ":a/main")
                .count(),
            3
        );
        assert!(e.observations.values().all(|o| o.source_ids.is_empty()));
    }
    #[test]
    fn library_artifact_projection_refuses_false_unique_owner_and_changed_identity() {
        for mutate in 0..4 {
            let mut input = metadata();
            match mutate {
                0 => {
                    input["binaryCallTargets"][0]["owners"][0]["artifact"]["contentDigest"] =
                        json!(format!("sha256:{}", "b".repeat(64)))
                }
                1 => {
                    input["binaryCallTargets"][0]["owners"][0]["classEntry"] =
                        json!("lookalike/Policy.class")
                }
                2 => input["binaryCallTargets"][0]["jvmDescriptor"] = json!("()V"),
                _ => input["binaryCallTargets"][0]["owners"][0]["methodPresent"] = json!(false),
            }
            assert!(project(&mut evidence(), &input, "payload", ":/main").is_err());
        }
    }
}
