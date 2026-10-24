//! Exact retained anchors. A binding slot is local to a protected human page.
use super::*;
use std::collections::BTreeSet;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Binding {
    pub schema: String,
    pub id: String,
    pub root_namespace: String,
    pub repository_id: String,
    pub service: String,
    pub provider: String,
    pub profile: String,
    pub compilations: Vec<String>,
    pub snapshot: String,
    pub revision: String,
    pub source_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observation_id: Option<String>,
    pub start_line: u64,
    pub end_line: u64,
    pub text_digest: String,
}

pub fn namespace(repo: &Repository) -> Result<String, ClewError> {
    digest(&repo.root.to_string_lossy())
}

pub fn validate(bindings: &[Binding], service: &str) -> Result<(), ClewError> {
    let mut ids = BTreeSet::new();
    if bindings.len() > 32 {
        return Err(invalid("protected page source bindings exceed 32 slots"));
    }
    for b in bindings {
        if b.schema != "codeclew-note-source-binding/1.0"
            || !store::valid_id(&b.id)
            || !ids.insert(&b.id)
            || b.service != service
            || !store::valid_id(&b.repository_id)
            || b.provider.is_empty()
            || b.profile.is_empty()
            || b.snapshot.is_empty()
            || b.revision.is_empty()
            || b.source_id.is_empty()
            || b.start_line == 0
            || b.end_line < b.start_line
            || !b.text_digest.starts_with("sha256:")
            || !b.root_namespace.starts_with("sha256:")
        {
            return Err(invalid("invalid typed note source binding or namespace"));
        }
    }
    Ok(())
}

fn scope(b: &Binding, service: &Service, evidence: &ServiceEvidence) -> bool {
    b.service == service.id
        && b.repository_id == service.repository_id
        && b.profile == service.profile
        && b.compilations == service.compilations
        && b.provider == evidence.extractor
}

fn selected(b: &Binding, evidence: &ServiceEvidence) -> Result<Source, ClewError> {
    let source = evidence
        .sources
        .get(&b.source_id)
        .ok_or_else(|| invalid("NOTE_BINDING_MISSING: selected retained source does not exist"))?;
    if source.id != b.source_id
        || source.service != b.service
        || source.revision != b.revision
        || source.start_line > b.start_line
        || source.end_line < b.end_line
        || crate::canonical::hash_bytes(source.text.as_bytes()) != source.text_digest
    {
        return Err(invalid(
            "NOTE_BINDING_SOURCE: revision, range or source digest differs",
        ));
    }
    if let Some(id) = &b.observation_id {
        let observation = evidence
            .observations
            .get(id)
            .ok_or_else(|| invalid("NOTE_BINDING_MISSING: selected Observation is absent"))?;
        if observation.service != b.service || !observation.source_ids.contains(&b.source_id) {
            return Err(invalid(
                "NOTE_BINDING_SCOPE: Observation does not own selected source",
            ));
        }
    }
    let text = subset(source, b.start_line, b.end_line)?;
    if crate::canonical::hash_bytes(text.as_bytes()) != b.text_digest {
        return Err(invalid(
            "NOTE_BINDING_RANGE_DIGEST: selected exact lines differ",
        ));
    }
    Ok(source.clone())
}

fn subset(source: &Source, start: u64, end: u64) -> Result<String, ClewError> {
    if start < source.start_line || end > source.end_line || end < start {
        return Err(invalid("source binding range is outside retained evidence"));
    }
    let lines: Vec<_> = source.text.split('\n').collect();
    let first = (start - source.start_line) as usize;
    let last = (end - source.start_line) as usize;
    lines
        .get(first..=last)
        .map(|s| s.join("\n"))
        .ok_or_else(|| invalid("source binding range has no retained text"))
}

pub fn admit(repo: &Repository, bindings: &[Binding]) -> Result<Vec<Value>, ClewError> {
    let mut snapshots = BTreeMap::new();
    let mut result = Vec::new();
    for b in bindings {
        if b.root_namespace != namespace(repo)? {
            return Err(invalid(
                "NOTE_BINDING_NAMESPACE: foreign documentation root",
            ));
        }
        if !snapshots.contains_key(&b.snapshot) {
            snapshots.insert(b.snapshot.clone(), Check::load_snapshot(repo, &b.snapshot)?);
        }
        let checked = &snapshots[&b.snapshot];
        let service = checked
            .source_inputs
            .as_ref()
            .and_then(|s| s.inputs.services.get(&b.service))
            .ok_or_else(|| {
                invalid("NOTE_BINDING_SCOPE: selected snapshot has no service inputs")
            })?;
        let evidence = checked
            .services
            .get(&b.service)
            .ok_or_else(|| invalid("NOTE_BINDING_SCOPE: selected service is unavailable"))?;
        if !scope(b, service, evidence) {
            return Err(invalid(
                "NOTE_BINDING_SCOPE: repository, provider, profile or compilation differs",
            ));
        }
        let source = selected(b, evidence)?;
        result.push(
            json!({"id":b.id,"state":"BOUND_UNCHANGED","selection":b,"selectedSource":source,
            "selectedText":subset(&source,b.start_line,b.end_line)?,"authority":source.authority,
            "influenceScope":"CONSERVATIVE_SERVICE","proseAssessment":"UNASSESSED"}),
        );
    }
    Ok(result)
}

pub fn resolve(
    b: &Binding,
    retained: &Value,
    checked: &Check,
    inputs: &store::RepositoryInputs,
) -> Value {
    let mut row = retained.clone();
    let state = match (
        inputs.services.get(&b.service),
        checked.services.get(&b.service),
    ) {
        (Some(service), Some(evidence)) if scope(b, service, evidence) => {
            let source = evidence.sources.get(&b.source_id);
            let observation = b
                .observation_id
                .as_ref()
                .map(|id| evidence.observations.get(id));
            if observation.is_some_and(|o| o.is_none()) {
                "MISSING"
            } else if observation
                .flatten()
                .is_some_and(|o| !o.source_ids.contains(&b.source_id))
            {
                "AMBIGUOUS"
            } else if let Some(source) = source {
                let old_start = retained["selectedSource"]["startLine"]
                    .as_u64()
                    .unwrap_or(b.start_line);
                let start = source.start_line + b.start_line.saturating_sub(old_start);
                let end = start + b.end_line - b.start_line;
                row["currentSource"] = json!(source);
                row["currentRange"] = json!({"startLine":start,"endLine":end});
                if subset(source, start, end)
                    .is_ok_and(|s| crate::canonical::hash_bytes(s.as_bytes()) == b.text_digest)
                {
                    if source.file != retained["selectedSource"]["file"]
                        || start != b.start_line
                        || end != b.end_line
                    {
                        "RELOCATED_ONLY"
                    } else {
                        "BOUND_UNCHANGED"
                    }
                } else {
                    "SOURCE_CHANGED"
                }
            } else {
                "MISSING"
            }
        }
        (Some(_), Some(_)) => "OUT_OF_SCOPE",
        _ => "UNAVAILABLE",
    };
    row["state"] = json!(state);
    row
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Binding, Service, ServiceEvidence, Source) {
        let source:Source=serde_json::from_value(json!({"id":"method-source","service":"orders","revision":"r1","file":"Orders.java","startLine":5,"endLine":6,"text":"first\nsecond","textDigest":crate::canonical::hash_bytes(b"first\nsecond"),"evidenceDigest":"evidence","authority":"SOURCE_SYNTAX","url":null})).unwrap();
        let service:Service=serde_json::from_value(json!({"schema":"codeclew-documentation-service/1.0","id":"orders","title":"Orders","repositoryId":"repo-orders","repository":"fixture","language":"java","profile":"source","targetRef":"HEAD","contractFiles":[]})).unwrap();
        let evidence = ServiceEvidence {
            schema: "fixture".into(),
            service: "orders".into(),
            revision: "r1".into(),
            service_digest: "fixture".into(),
            extractor: "syntax-v1".into(),
            runtime_mode: "SOURCE_SYNTAX".into(),
            coverage: "BOUNDED".into(),
            boundaries: vec![],
            entrypoints: vec![],
            observations: BTreeMap::from([(
                "method".into(),
                Observation {
                    id: "method".into(),
                    service: "orders".into(),
                    symbol: "Orders.run".into(),
                    kind: "SYMBOL".into(),
                    digest: "semantic".into(),
                    normalized: json!({}),
                    source_ids: vec![source.id.clone()],
                },
            )]),
            sources: BTreeMap::from([(source.id.clone(), source.clone())]),
            contracts: BTreeMap::new(),
        };
        let binding = Binding {
            schema: "codeclew-note-source-binding/1.0".into(),
            id: "entry".into(),
            root_namespace: "sha256:root".into(),
            repository_id: service.repository_id.clone(),
            service: service.id.clone(),
            provider: evidence.extractor.clone(),
            profile: service.profile.clone(),
            compilations: vec![],
            snapshot: "snapshot".into(),
            revision: "r1".into(),
            source_id: source.id.clone(),
            observation_id: Some("method".into()),
            start_line: 5,
            end_line: 6,
            text_digest: source.text_digest.clone(),
        };
        (binding, service, evidence, source)
    }
    #[test]
    fn exact_selection_rejects_wrong_revision_range_digest_and_observation() {
        let (b, _, e, _) = fixture();
        selected(&b, &e).unwrap();
        let mut wrong = b.clone();
        wrong.revision = "r2".into();
        assert!(selected(&wrong, &e).is_err());
        wrong = b.clone();
        wrong.end_line = 7;
        assert!(selected(&wrong, &e).is_err());
        wrong = b.clone();
        wrong.text_digest = crate::canonical::hash_bytes(b"different");
        assert!(selected(&wrong, &e).is_err());
        wrong = b.clone();
        wrong.observation_id = Some("another-compilation".into());
        assert!(selected(&wrong, &e).is_err());
    }
    #[test]
    fn relocation_uses_exact_id_and_preserves_old_evidence_without_rename_guess() {
        let (b, s, mut e, source) = fixture();
        let dir = tempfile::tempdir().unwrap();
        Repository::init(dir.path(), "Fixture").unwrap();
        let repo = Repository::open(dir.path()).unwrap();
        let mut inputs = repo.inputs().unwrap();
        inputs.services.insert(s.id.clone(), s);
        let mut checked:Check=serde_json::from_value(json!({"schema":"fixture","inputDigest":"i","contextDigest":"c","services":{},"unresolved":{},"interactions":{},"scenarios":{},"dependencies":{}})).unwrap();
        let old = json!({"id":b.id,"selection":b,"selectedSource":source,"selectedText":source.text,"state":"BOUND_UNCHANGED"});
        let moved = e.sources.get_mut(&b.source_id).unwrap();
        moved.start_line += 10;
        moved.end_line += 10;
        moved.file = "Moved.java".into();
        moved.revision = "r2".into();
        checked.services.insert("orders".into(), e.clone());
        let row = resolve(&b, &old, &checked, &inputs);
        assert_eq!(row["state"], "RELOCATED_ONLY");
        assert_eq!(row["selectedSource"], old["selectedSource"]);
        assert_eq!(row["currentRange"]["startLine"], 15);
        let changed = checked
            .services
            .get_mut("orders")
            .unwrap()
            .sources
            .get_mut(&b.source_id)
            .unwrap();
        changed.text = "changed\nsecond".into();
        changed.text_digest = crate::canonical::hash_bytes(changed.text.as_bytes());
        assert_eq!(
            resolve(&b, &old, &checked, &inputs)["state"],
            "SOURCE_CHANGED"
        );
        checked
            .services
            .get_mut("orders")
            .unwrap()
            .observations
            .clear();
        assert_eq!(resolve(&b, &old, &checked, &inputs)["state"], "MISSING");
    }
    #[test]
    fn foreign_root_and_wrong_provider_profile_are_not_admitted() {
        let (b, mut s, e, _) = fixture();
        let dir = tempfile::tempdir().unwrap();
        Repository::init(dir.path(), "Fixture").unwrap();
        let repo = Repository::open(dir.path()).unwrap();
        assert!(admit(&repo, &[b.clone()]).is_err());
        assert!(scope(&b, &s, &e));
        s.profile = "other".into();
        assert!(!scope(&b, &s, &e));
    }
    #[test]
    fn legacy_association_omits_new_fields_without_identity_drift() {
        let old = json!({"schema":"codeclew-documentation-note-association/1.0","id":"policy","title":"Policy","service":"orders","path":"notes/policy.md","targets":["service:orders"],"classification":"policy","period":"Current","tags":[],"metadata":{}});
        let association: Association = serde_json::from_value(old.clone()).unwrap();
        assert_eq!(serde_json::to_value(association).unwrap(), old);
    }
}
