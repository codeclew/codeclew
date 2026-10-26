//! Retain static JVM resources from the exact admitted repository snapshot.
//! Resource text is file evidence only, never resolved configuration or compiler semantics.
use super::super::model::{SOURCE_EXTRACTOR, Service, ServiceEvidence, Source, SourceOccurrence};
use super::{digest, invalid};
use crate::{canonical, error::ClewError};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

pub(super) fn retain(
    service: &Service,
    evidence: &mut ServiceEvidence,
    snapshot: &str,
    files: &BTreeMap<String, Arc<str>>,
    selected: &BTreeSet<String>,
) -> Result<(), ClewError> {
    if !matches!(service.language.as_str(), "java" | "kotlin") {
        return Ok(());
    }
    evidence.boundaries.push(
        "STATIC_CONFIGURATION_SCOPE:VERSIONED_STANDARD_SELECTED_JVM_RESOURCE_SIBLINGS_YAML_PROPERTIES_ONLY"
            .into(),
    );
    // Derive the standard resource sibling from actual admitted source paths.
    // Project labels need not match physical module directories.
    let roots = selected
        .iter()
        .filter_map(|path| {
            let absolute = format!("/{path}");
            let (module, tail) = absolute.split_once("/src/")?;
            let (set, language) = tail.split_once('/')?;
            if !matches!(set, "main" | "test")
                || !(language.starts_with("java/") || language.starts_with("kotlin/"))
            {
                return None;
            }
            Some(
                format!("{module}/src/{set}/resources/")
                    .trim_start_matches('/')
                    .to_owned(),
            )
        })
        .collect::<BTreeSet<_>>();
    let mut bytes = 0usize;
    let mut count = 0usize;
    for (path, text) in files {
        if !roots.iter().any(|root| path.starts_with(root))
            || ![".yml", ".yaml", ".properties"]
                .iter()
                .any(|suffix| path.ends_with(suffix))
        {
            continue;
        }
        bytes = bytes.saturating_add(text.len());
        count += 1;
        if text.len() > 2 * 1024 * 1024 || bytes > 32 * 1024 * 1024 || count > 2048 {
            return Err(invalid(
                "static configuration resource evidence exceeds bounded scope",
            ));
        }
        if text.is_empty() {
            continue;
        }
        let id = super::source_id(&service.id, &format!("configuration-resource:{path}"))?;
        let text_digest = canonical::hash_bytes(text.as_bytes());
        let end_line = text.lines().count() as u64;
        evidence.sources.insert(
            id.clone(),
            Source {
                id,
                service: service.id.clone(),
                revision: evidence.revision.clone(),
                file: path.clone(),
                start_line: 1,
                end_line,
                text: text.to_string(),
                text_digest: text_digest.clone(),
                evidence_digest: digest(&(
                    SOURCE_EXTRACTOR,
                    snapshot,
                    &text_digest,
                    0usize,
                    text.len(),
                ))?,
                authority: "EXACT_SNAPSHOT_TEXT".into(),
                text_origin: None,
                occurrence: Some(SourceOccurrence {
                    snapshot: snapshot.into(),
                    blob: text_digest,
                    start_byte: 0,
                    end_byte: text.len(),
                    generated_origin: None,
                }),
                url: super::source_link(service, &evidence.revision, path, 1, end_line),
            },
        );
        evidence
            .boundaries
            .push(format!("STATIC_CONFIGURATION_FILE_ONLY:{path}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn service(language: &str) -> Service {
        serde_json::from_value(json!({"schema":"codeclew-documentation-service/1.0","id":"calibration","title":"Calibration","repositoryId":"calibration","repository":"https://example.invalid/calibration.git","language":language,"profile":"test","targetRef":"main"})).unwrap()
    }
    fn capture(
        language: &str,
        revision: &str,
        files: &BTreeMap<String, Arc<str>>,
    ) -> ServiceEvidence {
        let s = service(language);
        let mut e = ServiceEvidence {
            schema: "codeclew-documentation-service-evidence/1.0".into(),
            service: s.id.clone(),
            revision: revision.into(),
            service_digest: "service".into(),
            extractor: crate::documentation::model::EXTRACTOR.into(),
            runtime_mode: "RELEASE".into(),
            coverage: "COMPILER".into(),
            boundaries: vec![],
            entrypoints: vec![],
            observations: BTreeMap::new(),
            sources: BTreeMap::new(),
            contracts: BTreeMap::new(),
        };
        let extension = if language == "java" {
            "java/Calibration.java"
        } else {
            "kotlin/Calibration.kt"
        };
        retain(
            &s,
            &mut e,
            revision,
            files,
            &BTreeSet::from([format!("modules/measure/src/main/{extension}")]),
        )
        .unwrap();
        crate::documentation::analysis::verify_evidence(&e).unwrap();
        e
    }
    #[test]
    fn java_and_kotlin_keep_exact_selected_module_resources_without_resolving_placeholders() {
        let files = BTreeMap::from([
            (
                "modules/measure/src/main/resources/application.yml".into(),
                Arc::from("client:\n  endpoint: ${CALIBRATION_ENDPOINT}\n"),
            ),
            (
                "modules/measure/src/main/resources/application-prod.yaml".into(),
                Arc::from("client:\n  enabled: ${CALIBRATION_ENABLED:false}\n"),
            ),
            (
                "modules/measure/src/main/resources/application.properties".into(),
                Arc::from("client.timeout=40\n"),
            ),
            (
                "modules/other/src/main/resources/application.yml".into(),
                Arc::from("unselected: true"),
            ),
            (
                "modules/measure/src/test/resources/application.yml".into(),
                Arc::from("test: true"),
            ),
            ("application.yml".into(), Arc::from("outside: true")),
        ]);
        for language in ["java", "kotlin"] {
            let e = capture(language, &"a".repeat(40), &files);
            assert_eq!(e.sources.len(), 3);
            assert_eq!(e.coverage, "COMPILER");
            for source in e.sources.values() {
                assert_eq!(source.text.as_str(), files[&source.file].as_ref());
                assert_eq!(
                    source.text_digest,
                    canonical::hash_bytes(source.text.as_bytes())
                );
                assert_eq!(source.revision, "a".repeat(40));
                assert_eq!(source.authority, "EXACT_SNAPSHOT_TEXT");
                assert_eq!(source.occurrence.as_ref().unwrap().snapshot, "a".repeat(40));
            }
            assert!(e.observations.is_empty());
        }
    }
    #[test]
    fn changed_and_removed_configuration_cannot_survive_the_next_capture() {
        let path = "modules/measure/src/main/resources/application.yml".to_owned();
        let mut files = BTreeMap::from([(path.clone(), Arc::from("enabled: false\n"))]);
        let first = capture("java", &"a".repeat(40), &files);
        files.insert(path.clone(), Arc::from("enabled: true\n"));
        let changed = capture("java", &"b".repeat(40), &files);
        let a = first.sources.values().next().unwrap();
        let b = changed.sources.values().next().unwrap();
        assert_eq!(a.id, b.id);
        assert_ne!(a.text_digest, b.text_digest);
        assert_ne!(a.evidence_digest, b.evidence_digest);
        assert_eq!(b.text, "enabled: true\n");
        assert_eq!(b.revision, "b".repeat(40));
        files.remove(&path);
        assert!(capture("java", &"c".repeat(40), &files).sources.is_empty());
    }
}
