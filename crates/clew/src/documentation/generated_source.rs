//! Explicit existing generated inputs; no build, directory sweep, or Git authority.
use super::{digest, invalid, io_error, model::GeneratedInput, store};
use crate::{canonical, cas::CasStore, error::ClewError, repository_snapshot::IndexEntry};
use serde_json::json;
use std::{path::Path, process::Command};
const MAX_FILE: usize = 2 * 1024 * 1024;
pub(super) fn receipt(path: &str, origin: &str, content_digest: &str) -> Result<String, ClewError> {
    digest(
        &json!({"schema":"codeclew-generated-source-input/1.0","path":path,"origin":origin,"contentDigest":content_digest}),
    )
}
pub(super) fn admit(
    repo: &Path,
    revision: &str,
    inputs: &[GeneratedInput],
    cas: &CasStore,
) -> Result<Vec<(IndexEntry, String, String)>, ClewError> {
    let mut result = Vec::new();
    let mut total = 0usize;
    for input in inputs {
        store::relative(&input.path)?;
        let tracked = Command::new("git")
            .current_dir(repo)
            .args([
                "--literal-pathspecs",
                "ls-tree",
                "-z",
                revision,
                "--",
                &input.path,
            ])
            .output()
            .map_err(io_error)?;
        if !tracked.status.success() || !tracked.stdout.is_empty() {
            return Err(invalid(format!(
                "GENERATED_INPUT_COMMITTED_PATH_COLLISION:{}",
                input.path
            )));
        }
        let bytes = crate::repository_snapshot::read_documentation_generated_input(
            repo,
            &input.path,
            MAX_FILE as u64,
        )?;
        total = total
            .checked_add(bytes.len())
            .ok_or_else(|| invalid("generated input budget exceeded"))?;
        if total > 32 * 1024 * 1024 {
            return Err(invalid("generated input budget exceeded"));
        }
        let content_digest = canonical::hash_bytes(&bytes);
        let snapshot = receipt(&input.path, &input.origin, &content_digest)?;
        let content = cas.put("codeclew-generated-source-input-bytes/1.0", &bytes)?;
        result.push((
            IndexEntry {
                path: input.path.clone(),
                mode: 0o100644,
                stage: 0,
                git_oid: format!("generated:{content_digest}"),
                content,
            },
            snapshot,
            input.origin.clone(),
        ));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn repo() -> (tempfile::TempDir, tempfile::TempDir, CasStore, String) {
        let repo = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        for args in [
            &["init", "-q"][..],
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "--allow-empty",
                "-qm",
                "Fixture",
            ][..],
        ] {
            assert!(
                Command::new("git")
                    .current_dir(repo.path())
                    .args(args)
                    .status()
                    .unwrap()
                    .success()
            );
        }
        let revision = super::super::analysis::git(repo.path(), &["rev-parse", "HEAD"]).unwrap();
        let cas = CasStore::open(
            &crate::state::StateAuthority::open(state.path().join("state")).unwrap(),
        )
        .unwrap();
        (repo, state, cas, revision)
    }
    #[test]
    fn explicit_bytes_and_origin_change_receipts_without_a_git_commit() {
        let (repo, _state, cas, rev) = repo();
        std::fs::write(repo.path().join("Generated.java"), "class Generated {}").unwrap();
        let input = GeneratedInput {
            path: "Generated.java".into(),
            origin: "Fixture generator output".into(),
        };
        let a = admit(repo.path(), &rev, std::slice::from_ref(&input), &cas).unwrap();
        std::fs::write(
            repo.path().join("Generated.java"),
            "class Generated { int n; }",
        )
        .unwrap();
        let b = admit(repo.path(), &rev, std::slice::from_ref(&input), &cas).unwrap();
        assert_ne!(a[0].1, b[0].1);
        assert_ne!(a[0].0.content, b[0].0.content);
        let mut relabeled = input.clone();
        relabeled.origin = "Different explicit generator".into();
        let c = admit(repo.path(), &rev, &[relabeled], &cas).unwrap();
        assert_eq!(b[0].0.content, c[0].0.content);
        assert_ne!(b[0].1, c[0].1);
        std::fs::remove_file(repo.path().join("Generated.java")).unwrap();
        assert!(
            admit(repo.path(), &rev, &[input], &cas)
                .unwrap_err()
                .message
                .contains("MISSING")
        );
        assert_eq!(
            cas.read(&b[0].0.content, MAX_FILE).unwrap().bytes(),
            b"class Generated { int n; }"
        );
    }
    #[test]
    fn committed_paths_unsafe_components_directories_and_fifo_fail_before_capture() {
        let (repo, _state, cas, _rev) = repo();
        std::fs::write(repo.path().join("Committed.java"), "class C {}").unwrap();
        assert!(
            Command::new("git")
                .current_dir(repo.path())
                .args(["add", "Committed.java"])
                .status()
                .unwrap()
                .success()
        );
        assert!(
            Command::new("git")
                .current_dir(repo.path())
                .args([
                    "-c",
                    "user.name=Fixture",
                    "-c",
                    "user.email=fixture@example.invalid",
                    "commit",
                    "-qm",
                    "Tracked"
                ])
                .status()
                .unwrap()
                .success()
        );
        let rev = super::super::analysis::git(repo.path(), &["rev-parse", "HEAD"]).unwrap();
        for path in ["Committed.java", "../escape.java", "/absolute.java"] {
            assert!(
                admit(
                    repo.path(),
                    &rev,
                    &[GeneratedInput {
                        path: path.into(),
                        origin: "Fixture".into()
                    }],
                    &cas
                )
                .is_err(),
                "{path}"
            );
        }
        std::fs::create_dir(repo.path().join("directory")).unwrap();
        assert!(
            admit(
                repo.path(),
                &rev,
                &[GeneratedInput {
                    path: "directory".into(),
                    origin: "Fixture".into()
                }],
                &cas
            )
            .is_err()
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let outside = tempfile::tempdir().unwrap();
            std::fs::write(outside.path().join("Escape.java"), "class E {}").unwrap();
            symlink(outside.path(), repo.path().join("link")).unwrap();
            assert!(
                admit(
                    repo.path(),
                    &rev,
                    &[GeneratedInput {
                        path: "link/Escape.java".into(),
                        origin: "Fixture".into()
                    }],
                    &cas
                )
                .is_err()
            );
            let fifo =
                std::ffi::CString::new(repo.path().join("pipe.java").to_str().unwrap()).unwrap();
            assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
            let started = std::time::Instant::now();
            assert!(
                admit(
                    repo.path(),
                    &rev,
                    &[GeneratedInput {
                        path: "pipe.java".into(),
                        origin: "Fixture".into()
                    }],
                    &cas
                )
                .is_err()
            );
            assert!(started.elapsed() < std::time::Duration::from_secs(2));
        }
    }
}
