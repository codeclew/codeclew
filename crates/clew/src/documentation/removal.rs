//! Remove only an unreferenced live catalogue declaration; frozen history stays
//! immutable. A current-root digest binds the reviewed impact to application.
use super::{
    digest, invalid, io_error,
    store::{self, Repository},
};
use crate::error::{ClewError, ErrorCode};
use clap::Args;
use serde_json::{Value, json};
use std::path::PathBuf;

#[derive(Debug, Args)]
pub struct Request {
    #[arg(long)]
    pub root: PathBuf,
    #[arg(long)]
    pub id: String,
    #[arg(long)]
    pub expected_input_digest: Option<String>,
    /// Without this flag, return reference impact only.
    #[arg(long)]
    pub apply: bool,
}

fn mentions(value: &Value, id: &str, subject: &str) -> bool {
    match value {
        Value::String(s) => s == id || s == subject || s.starts_with(&format!("{subject}/")),
        Value::Array(v) => v.iter().any(|v| mentions(v, id, subject)),
        Value::Object(v) => v
            .iter()
            .any(|(k, v)| k == subject || mentions(v, id, subject)),
        _ => false,
    }
}

pub fn run(kind: &str, args: Request) -> Result<Value, ClewError> {
    if !store::valid_id(&args.id) {
        return Err(invalid("invalid catalogue identity"));
    }
    let repo = Repository::open(&args.root)?;
    let _lock = repo.lock()?;
    let inputs = repo.inputs()?;
    let current = digest(&inputs)?;
    let folder = match kind {
        "service" => "services",
        "entity" => "entities",
        "external-service" => "external-services",
        _ => return Err(invalid("unsupported removal kind")),
    };
    let path = repo.path(&format!("catalog/{folder}/{}.json", args.id))?;
    let subject = format!("{kind}:{}", args.id);
    let mut value = serde_json::to_value(&inputs).map_err(io_error)?;
    let input_field = if kind == "external-service" {
        "externalServices"
    } else {
        folder
    };
    value[input_field]
        .as_object_mut()
        .ok_or_else(|| invalid("invalid catalogue inputs"))?
        .remove(&args.id);
    // Conservatively report each current catalogue record mentioning this stable
    // identity. It is a reviewable blocker, never an implicit cascading edit.
    let mut references = Vec::new();
    for (category, rows) in value.as_object().into_iter().flatten() {
        if let Some(rows) = rows.as_object() {
            for (id, row) in rows {
                if mentions(row, &args.id, &subject) {
                    references.push(format!("{category}:{id}"));
                }
            }
        }
    }
    // Live authored material outside RepositoryInputs is also a root. Historical
    // publications and immutable Work remain readable using frozen declarations.
    for directory in ["narratives", "catalog/views", "catalog/processes"] {
        let parent = repo.path(directory)?;
        if !parent.exists() {
            continue;
        }
        for entry in walkdir::WalkDir::new(&parent).follow_links(false) {
            let entry = entry.map_err(io_error)?;
            let relative = entry
                .path()
                .strip_prefix(&repo.root)
                .map_err(io_error)?
                .to_str()
                .ok_or_else(|| invalid("invalid authored path"))?;
            let path = repo.path(relative)?;
            if entry.file_type().is_file() {
                let record: Value = store::read(&path, store::MAX_RECORD)?;
                if mentions(&record, &args.id, &subject) {
                    references.push(relative.into());
                }
            }
        }
    }
    references.sort();
    references.dedup();
    if args.apply {
        if args.expected_input_digest.as_deref() != Some(&current) {
            return Err(ClewError::new(
                ErrorCode::WwConflict,
                "catalogue changed; review removal impact and inputDigest again",
            ));
        }
        if !references.is_empty() {
            return Err(invalid(format!(
                "removal blocked by current references: {}; update those declarations explicitly; no cascade performed",
                references.join(", ")
            )));
        }
        if path.exists() {
            std::fs::remove_file(path).map_err(io_error)?;
        }
    }
    Ok(
        json!({"schema":"codeclew-documentation-removal/1.0","subject":subject,"status":if args.apply {"REMOVED"} else if references.is_empty(){"REMOVABLE"}else{"BLOCKED"},"references":references,"inputDigest":repo.input_digest()?,"history":"Frozen publications, proposals, Work and snapshots retain their original declarations; no historical rewrite or physical reclamation performed."}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn service_removal_reports_references_and_preserves_frozen_history() {
        let t = tempfile::tempdir().unwrap();
        Repository::init(t.path(), "Removal").unwrap();
        let repo = Repository::open(t.path()).unwrap();
        let service:super::super::model::Service=serde_json::from_value(json!({"schema":"codeclew-documentation-service/1.0","id":"weather","title":"Weather","repositoryId":"owned","repository":"https://github.com/owned/weather.git","language":"java","targetRef":"main","profile":"source-syntax","source":{"roots":["src"],"dialect":"17"}})).unwrap();
        repo.service_add(service, Some(&repo.input_digest().unwrap()))
            .unwrap();
        let entity = json!({"schema":"codeclew-documentation-entity/1.0","id":"city","title":"City","description":"Saved city","relations":[{"service":"weather","kind":"owned","origin":"human","rationale":"Explicit owner","confidence":"declared"}],"limitations":[]});
        repo.atomic(
            "catalog/entities/city.json",
            &super::super::bytes(&entity).unwrap(),
        )
        .unwrap();
        let history = b"frozen historical declaration";
        repo.atomic("docs/generated/old/service.txt", history)
            .unwrap();
        let request = |apply| Request {
            root: t.path().into(),
            id: "weather".into(),
            expected_input_digest: Some(repo.input_digest().unwrap()),
            apply,
        };
        let impact = run("service", request(false)).unwrap();
        assert_eq!(impact["status"], "BLOCKED");
        assert_eq!(impact["references"], json!(["entities:city"]));
        assert!(run("service", request(true)).is_err());
        run(
            "entity",
            Request {
                root: t.path().into(),
                id: "city".into(),
                expected_input_digest: Some(repo.input_digest().unwrap()),
                apply: true,
            },
        )
        .unwrap();
        let result = run("service", request(true)).unwrap();
        assert_eq!(result["status"], "REMOVED");
        assert!(repo.services().unwrap().is_empty());
        assert_eq!(
            std::fs::read(repo.path("docs/generated/old/service.txt").unwrap()).unwrap(),
            history
        );
    }
    #[test]
    fn related_entity_and_stale_digest_block_deletion() {
        let t = tempfile::tempdir().unwrap();
        Repository::init(t.path(), "Removal").unwrap();
        let repo = Repository::open(t.path()).unwrap();
        for (id, related) in [("city", vec![]), ("forecast", vec!["city"])] {
            repo.atomic(&format!("catalog/entities/{id}.json"),&super::super::bytes(&json!({"schema":"codeclew-documentation-entity/1.0","id":id,"title":id,"description":"Owned","relations":[],"relatedEntities":related,"limitations":[]})).unwrap()).unwrap();
        }
        let impact = run(
            "entity",
            Request {
                root: t.path().into(),
                id: "city".into(),
                expected_input_digest: None,
                apply: false,
            },
        )
        .unwrap();
        assert_eq!(impact["status"], "BLOCKED");
        assert_eq!(
            run(
                "entity",
                Request {
                    root: t.path().into(),
                    id: "forecast".into(),
                    expected_input_digest: Some("stale".into()),
                    apply: true
                }
            )
            .unwrap_err()
            .code,
            ErrorCode::WwConflict
        );
    }
}
