//! Committed byte provenance, independent of compiler/syntax semantic authority.
use super::{
    digest, invalid,
    model::{CommittedTextOrigin, Source, SourceOccurrence},
};
use crate::{
    canonical,
    cas::CasStore,
    error::ClewError,
    repository_snapshot::{self, TrackedScopeLimits},
};
use std::{collections::BTreeMap, path::Path, sync::Arc};

pub(super) const AUDIT: &str = "ADMITTED_COMPILATION_SOURCE_AUDIT_NOT_CALL_RESOLUTION";
const SCHEMA: &str = "codeclew-committed-source-text/1.0";
const MAX_BYTES: usize = 32 * 1024 * 1024;

pub(super) struct CommittedFile {
    revision: String,
    path: String,
    git_blob: String,
    content_digest: String,
    snapshot_digest: String,
}

/// Read only selected immutable Git blobs through the existing bounded snapshot
/// machinery. A matching working tree or SHA-looking revision is not proof.
pub(super) fn capture(
    repo: &Path,
    revision: &str,
    inputs: &BTreeMap<String, Arc<str>>,
    store: &CasStore,
) -> Result<BTreeMap<String, CommittedFile>, ClewError> {
    let mut bytes = 0usize;
    let mut paths = 0usize;
    let selected: BTreeMap<_, _> = inputs
        .iter()
        .take(2048)
        .take_while(|(path, text)| {
            bytes = bytes.saturating_add(text.len());
            paths = paths.saturating_add(path.len() + 32);
            bytes <= MAX_BYTES && paths <= 128 * 1024
        })
        .map(|(p, t)| (p.clone(), t))
        .collect();
    if selected.is_empty() {
        return Ok(BTreeMap::new());
    }
    let roots = selected.keys().cloned().collect::<Vec<_>>();
    let (snapshot, object) = repository_snapshot::capture_documentation_scope(
        repo,
        revision,
        &roots,
        store,
        TrackedScopeLimits {
            max_files: 2048,
            max_file_bytes: MAX_BYTES,
            max_total_bytes: MAX_BYTES,
            max_tree_entries: 2048,
            max_tree_bytes: 2 * 1024 * 1024,
            max_tree_path_bytes: 4096,
        },
    )?;
    Ok(snapshot
        .index
        .into_iter()
        .filter_map(|entry| {
            let input = selected.get(&entry.path)?;
            // Dirty/staged bytes, untracked/generated files and symlinks cannot gain
            // committed origin merely by sharing a path or a selected revision.
            (matches!(entry.mode, 0o100644 | 0o100755)
                && entry.stage == 0
                && entry.content.size == input.len() as u64
                && crate::cas::CasObject::for_bytes(&entry.content.object_schema, input.as_bytes())
                    .is_ok_and(|expected| expected == entry.content))
            .then(|| {
                (
                    entry.path.clone(),
                    CommittedFile {
                        revision: revision.into(),
                        path: entry.path,
                        git_blob: entry.git_oid,
                        content_digest: canonical::hash_bytes(input.as_bytes()),
                        snapshot_digest: object.digest.clone(),
                    },
                )
            })
        })
        .collect())
}

pub(super) fn bind(
    source: &Source,
    occurrence: &SourceOccurrence,
    full_text: &str,
    file: &CommittedFile,
) -> Result<Option<CommittedTextOrigin>, ClewError> {
    if source.revision != file.revision
        || source.file != file.path
        || occurrence.generated_origin.is_some()
        || occurrence.blob.starts_with("generated:")
        || occurrence.start_byte >= occurrence.end_byte
        || canonical::hash_bytes(full_text.as_bytes()) != file.content_digest
        || full_text.get(occurrence.start_byte..occurrence.end_byte) != Some(source.text.as_str())
        || canonical::hash_bytes(source.text.as_bytes()) != source.text_digest
    {
        return Ok(None);
    }
    let start_line = 1 + full_text.as_bytes()[..occurrence.start_byte]
        .iter()
        .filter(|b| **b == b'\n')
        .count() as u64;
    let end_line = start_line + source.text.lines().count() as u64 - 1;
    if (source.start_line, source.end_line) != (start_line, end_line) {
        return Ok(None);
    }
    let mut origin = CommittedTextOrigin {
        schema: SCHEMA.into(),
        revision: source.revision.clone(),
        file: source.file.clone(),
        git_blob: file.git_blob.clone(),
        content_digest: file.content_digest.clone(),
        snapshot_digest: file.snapshot_digest.clone(),
        start_byte: occurrence.start_byte,
        end_byte: occurrence.end_byte,
        line_start: start_line,
        line_end: end_line,
        text_digest: source.text_digest.clone(),
        evidence_digest: source.evidence_digest.clone(),
        receipt: String::new(),
    };
    origin.receipt = digest(&origin)?;
    Ok(Some(origin))
}

fn sha(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|s| {
        s.len() == 64
            && s.bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    })
}

impl CommittedTextOrigin {
    pub(super) fn matches(
        &self,
        revision: &str,
        file: &str,
        text_digest: &str,
        evidence_digest: &str,
        start_line: u64,
        end_line: u64,
    ) -> bool {
        let mut unsigned = self.clone();
        unsigned.receipt.clear();
        self.schema == SCHEMA
            && self.revision == revision
            && self.file == file
            && self.text_digest == text_digest
            && self.evidence_digest == evidence_digest
            && self.line_start == start_line
            && self.line_end == end_line
            && start_line > 0
            && end_line >= start_line
            && self.revision.len() == 40
            && self.revision.bytes().all(|b| b.is_ascii_hexdigit())
            && matches!(self.git_blob.len(), 40 | 64)
            && self.git_blob.bytes().all(|b| b.is_ascii_hexdigit())
            && sha(&self.content_digest)
            && sha(&self.snapshot_digest)
            && sha(&self.text_digest)
            && sha(&self.evidence_digest)
            && self.end_byte > self.start_byte
            && self.end_byte - self.start_byte <= MAX_BYTES
            && digest(&unsigned).ok().as_ref() == Some(&self.receipt)
    }
}

pub(super) fn verify(source: &Source) -> Result<(), ClewError> {
    if let Some(origin) = &source.text_origin
        && (source.authority != AUDIT
            || source
                .occurrence
                .as_ref()
                .is_some_and(|o| o.generated_origin.is_some() || o.blob.starts_with("generated:"))
            || !origin.matches(
                &source.revision,
                &source.file,
                &source.text_digest,
                &source.evidence_digest,
                source.start_line,
                source.end_line,
            )
            || origin.end_byte - origin.start_byte != source.text.len())
    {
        return Err(invalid("committed source-text origin is inconsistent"));
    }
    Ok(())
}

pub(super) fn exact_audit(value: &serde_json::Value) -> bool {
    if value["authority"] != AUDIT
        || value["occurrence"]["generatedOrigin"].is_string()
        || value["occurrence"]["blob"]
            .as_str()
            .is_some_and(|b| b.starts_with("generated:"))
    {
        return false;
    }
    let Ok(origin) = serde_json::from_value::<CommittedTextOrigin>(value["textOrigin"].clone())
    else {
        return false;
    };
    let fields = ["revision", "file", "textDigest", "evidenceDigest"]
        .map(|key| value[key].as_str().unwrap_or(""));
    origin.matches(
        fields[0],
        fields[1],
        fields[2],
        fields[3],
        value["startLine"].as_u64().unwrap_or(0),
        value["endLine"].as_u64().unwrap_or(0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::StateAuthority;
    use serde_json::json;
    use std::{fs, process::Command};

    fn git(repo: &Path, args: &[&str]) -> String {
        let result = Command::new("git")
            .args(args)
            .current_dir(repo)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        String::from_utf8(result.stdout).unwrap().trim().into()
    }

    fn fixture() -> (
        tempfile::TempDir,
        tempfile::TempDir,
        CasStore,
        String,
        String,
    ) {
        let repo = tempfile::tempdir().unwrap();
        git(repo.path(), &["init", "-q"]);
        git(repo.path(), &["config", "core.autocrlf", "false"]);
        git(repo.path(), &["config", "user.name", "Test"]);
        git(
            repo.path(),
            &["config", "user.email", "test@example.invalid"],
        );
        let text =
            "// λ🔥 original\r\n@GetMapping(\"/owned\")\r\nvoid run() { save(); }\r\n".to_owned();
        fs::write(repo.path().join("A.java"), &text).unwrap();
        fs::write(repo.path().join("Dirty.java"), &text).unwrap();
        std::os::unix::fs::symlink("A.java", repo.path().join("Link.java")).unwrap();
        git(repo.path(), &["add", "."]);
        git(repo.path(), &["commit", "-qm", "owned committed sources"]);
        let revision = git(repo.path(), &["rev-parse", "HEAD"]);
        let state = tempfile::tempdir().unwrap();
        let authority = StateAuthority::open(state.path().join("v2")).unwrap();
        let store = CasStore::open(&authority).unwrap();
        (repo, state, store, revision, text)
    }

    fn source(revision: &str, full: &str) -> (Source, SourceOccurrence) {
        let start = full.find("@GetMapping").unwrap();
        let end = full.rfind("\r\n").unwrap();
        let exact = &full[start..end];
        (
            Source {
                id: "audit".into(),
                service: "svc".into(),
                revision: revision.into(),
                file: "A.java".into(),
                start_line: 2,
                end_line: 3,
                text: exact.into(),
                text_digest: canonical::hash_bytes(exact.as_bytes()),
                evidence_digest: canonical::hash_bytes(b"admitted compilation"),
                authority: AUDIT.into(),
                text_origin: None,
                occurrence: None,
                url: Some(format!(
                    "https://github.com/example/owned/blob/{revision}/A.java#L2-L3"
                )),
            },
            SourceOccurrence {
                snapshot: revision.into(),
                blob: canonical::hash_bytes(full.as_bytes()),
                start_byte: start,
                end_byte: end,
                generated_origin: None,
            },
        )
    }

    #[test]
    fn actual_git_bytes_admit_audit_links_without_promoting_semantic_authority() {
        let (repo, _state, store, revision, text) = fixture();
        let dirty = text.replace("original", "modified");
        fs::write(repo.path().join("Dirty.java"), &dirty).unwrap();
        let inputs = BTreeMap::from([
            ("A.java".into(), Arc::<str>::from(text.as_str())),
            ("Dirty.java".into(), Arc::from(dirty)),
            ("Untracked.java".into(), Arc::from(text.as_str())),
            ("Link.java".into(), Arc::from(text.as_str())),
        ]);
        let proof = capture(repo.path(), &revision, &inputs, &store).unwrap();
        assert_eq!(proof.keys().cloned().collect::<Vec<_>>(), vec!["A.java"]);
        let (mut source, occurrence) = source(&revision, &text);
        source.text_origin = bind(&source, &occurrence, &text, &proof["A.java"]).unwrap();
        verify(&source).unwrap();
        assert!(exact_audit(&json!(source)));
        let checked=serde_json::from_value(json!({"schema":"codeclew-documentation-check/1.0","inputDigest":"input","contextDigest":"context","services":{},"dependencies":{},"unresolved":{},"interactions":{},"scenarios":{}})).unwrap();
        let remote = super::super::source_export::Policy {
            mode: super::super::source_export::SourceMode::Remote,
            ..Default::default()
        };
        let projection = super::super::source_export::Projection::new(&remote, &checked).unwrap();
        let original = json!(source);
        let mut exported = original.clone();
        projection.value(&mut exported).unwrap();
        assert_eq!(exported["authority"], AUDIT);
        assert_eq!(exported["text"], "");
        assert_eq!(exported["textOrigin"], original["textOrigin"]);
        assert_eq!(exported["url"], original["url"]);
        let first = exported.clone();
        projection.value(&mut exported).unwrap();
        assert_eq!(exported, first);
        let embedded = super::super::source_export::Policy::default();
        let mut kept = original.clone();
        super::super::source_export::Projection::new(&embedded, &checked)
            .unwrap()
            .value(&mut kept)
            .unwrap();
        assert_eq!(kept, original);
        let mut legacy = source.clone();
        legacy.text_origin = None;
        verify(&legacy).unwrap();
        assert!(!exact_audit(&json!(legacy)));
        assert!(
            projection
                .value(&mut json!(legacy))
                .unwrap_err()
                .message
                .contains("committed-byte provenance")
        );
    }

    #[test]
    fn origin_rejects_wrong_utf8_span_line_text_revision_and_receipt() {
        let (repo, _state, store, revision, text) = fixture();
        let inputs = BTreeMap::from([("A.java".into(), Arc::<str>::from(text.as_str()))]);
        let proof = capture(repo.path(), &revision, &inputs, &store).unwrap();
        let (mut source, occurrence) = source(&revision, &text);
        source.text_origin = bind(&source, &occurrence, &text, &proof["A.java"]).unwrap();
        assert!(source.text_origin.is_some());
        for case in [
            "wrong-text",
            "wrong-line",
            "wrong-file",
            "wrong-revision",
            "utf8",
            "span",
            "generated",
        ] {
            let mut s = source.clone();
            s.text_origin = None;
            let mut range = occurrence.clone();
            match case {
                "wrong-text" => {
                    s.text = s.text.replace("save", "load");
                    s.text_digest = canonical::hash_bytes(s.text.as_bytes());
                }
                "wrong-file" => s.file = "Other.java".into(),
                "wrong-revision" => s.revision = "f".repeat(40),
                "wrong-line" => {
                    s.start_line = 1;
                    s.end_line = 2;
                }
                "utf8" => {
                    range.start_byte = text.find('λ').unwrap() + 1;
                }
                "span" => {
                    range.start_byte += 1;
                    range.end_byte += 1;
                }
                "generated" => range.generated_origin = Some("annotation processor".into()),
                _ => unreachable!(),
            }
            assert!(
                bind(&s, &range, &text, &proof["A.java"]).unwrap().is_none(),
                "{case}"
            );
        }
        for case in [
            "revision",
            "file",
            "evidence",
            "line",
            "receipt",
            "transformed",
            "generated",
        ] {
            let mut s = source.clone();
            match case {
                "revision" => s.revision = "b".repeat(40),
                "file" => s.file = "Other.java".into(),
                "evidence" => s.evidence_digest = canonical::hash_bytes(b"other evidence"),
                "line" => s.start_line += 1,
                "receipt" => {
                    s.text_origin.as_mut().unwrap().receipt = canonical::hash_bytes(b"damaged")
                }
                "transformed" => s.authority = "TRANSFORMED_SOURCE".into(),
                "generated" => {
                    s.occurrence = Some(SourceOccurrence {
                        generated_origin: Some("processor".into()),
                        ..occurrence.clone()
                    })
                }
                _ => unreachable!(),
            }
            assert!(verify(&s).is_err(), "{case}");
            assert!(!exact_audit(&json!(s)), "{case}");
        }
        let changed = text.replace("save", "load");
        let changed_inputs = BTreeMap::from([("A.java".into(), Arc::<str>::from(changed))]);
        assert!(
            capture(repo.path(), &revision, &changed_inputs, &store)
                .unwrap()
                .is_empty()
        );
    }
}
