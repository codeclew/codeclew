//! Narrow Git metadata authority for the isolated Kotlin/Gradle checkout.
use crate::{
    canonical,
    error::{ClewError, ErrorCode},
    repository_snapshot::isolated_git_command,
};
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Stdio,
};
use uuid::Uuid;

const MAX_REFS: usize = 512;
const MAX_REF_BYTES: usize = 512;
const MAX_OBJECTS: usize = 1024;
const MAX_OBJECT_BYTES: usize = 64 * 1024;
const MAX_TOTAL_BYTES: usize = 4 * 1024 * 1024;
const MAX_TAG_DEPTH: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Binding {
    pub id: Uuid,
    pub count: usize,
}

impl Binding {
    pub(crate) fn is_valid(&self) -> bool {
        self.id.get_version_num() == 4 && self.count <= MAX_REFS
    }
}

pub(crate) fn safe_summary(value: &serde_json::Value) -> Option<serde_json::Value> {
    let count = usize::try_from(value.get("count")?.as_u64()?).ok()?;
    let digest = value
        .get("inputBinding")?
        .as_str()?
        .strip_prefix("sha256:")?;
    (count <= MAX_REFS
        && value.get("source")?.as_str()? == "BOUND_LOCAL_GIT"
        && value.get("status")?.as_str()? == "APPLIED"
        && digest.len() == 64
        && digest.bytes().all(|byte| byte.is_ascii_hexdigit()))
    .then(|| serde_json::json!({"source":"BOUND_LOCAL_GIT","status":"APPLIED","count":count}))
}

// Only the binding is persisted. Names and object bytes stay in the owning
// repository and the short-lived compilation workspace.
pub(crate) struct LocalTags {
    pub binding: Binding,
    refs: Vec<(String, String)>,
    source: PathBuf,
}

impl LocalTags {
    pub(crate) fn read(
        repo: &Path,
        revision: &str,
        identity: impl FnOnce(&str, usize) -> Result<Binding, ClewError>,
    ) -> Result<Self, ClewError> {
        let refs = list_refs(repo, revision)?;
        let hash = canonical::hash(&(revision, &refs)).map_err(|_| unavailable())?;
        let binding = identity(&hash, refs.len())?;
        Ok(Self {
            binding,
            refs,
            source: repo.to_path_buf(),
        })
    }

    pub(crate) fn import(&self, destination: &Path) -> Result<(), ClewError> {
        let mut admitted = std::collections::BTreeMap::<String, usize>::new();
        let mut total = 0;
        for (_, oid) in &self.refs {
            let mut target = oid.clone();
            let mut expected_kind = None;
            let mut annotations = Vec::new();
            let remaining_depth = loop {
                if let Some(&depth) = admitted.get(&target) {
                    let kind = if depth == 0 { "commit" } else { "tag" };
                    if expected_kind.is_some_and(|expected| expected != kind) {
                        return Err(unavailable());
                    }
                    if annotations.len() + depth > MAX_TAG_DEPTH {
                        return Err(limit());
                    }
                    break depth;
                }
                if admitted.len() + annotations.len() >= MAX_OBJECTS {
                    return Err(limit());
                }
                let (kind, bytes) = read_object(&self.source, &target)?;
                if expected_kind.is_some_and(|expected| expected != kind) {
                    return Err(unavailable());
                }
                if annotations.len() == MAX_TAG_DEPTH && kind == "tag" {
                    return Err(limit());
                }
                total += bytes.len();
                if total > MAX_TOTAL_BYTES {
                    return Err(limit());
                }
                let imported = git(
                    destination,
                    &["hash-object", "-w", "-t", &kind, "--stdin"],
                    Some(&bytes),
                    128,
                )?;
                if imported.strip_suffix(b"\n") != Some(target.as_bytes()) {
                    return Err(unavailable());
                }
                if kind == "commit" {
                    admitted.insert(target, 0);
                    break 0;
                }
                let mut headers = bytes.split(|byte| *byte == b'\n');
                let next = headers
                    .next()
                    .and_then(|line| line.strip_prefix(b"object "))
                    .and_then(|oid| std::str::from_utf8(oid).ok())
                    .filter(|child| valid_oid(child, target.len()))
                    .ok_or_else(unavailable)?;
                expected_kind = match headers.next() {
                    Some(b"type tag") => Some("tag"),
                    Some(b"type commit") => Some("commit"),
                    _ => return Err(unavailable()),
                };
                annotations.push(target);
                target = next.to_owned();
            };
            for (index, oid) in annotations.into_iter().rev().enumerate() {
                admitted.insert(oid, remaining_depth + index + 1);
            }
        }
        if !self.refs.is_empty() {
            let mut updates = String::new();
            for (name, oid) in &self.refs {
                use std::fmt::Write;
                writeln!(updates, "create {name} {oid}").map_err(|_| unavailable())?;
            }
            git(
                destination,
                &["update-ref", "--stdin"],
                Some(updates.as_bytes()),
                128,
            )?;
        }
        // Check that Git retained the complete original ref namespace.
        if installed_refs(destination, &self.refs)? {
            return Ok(());
        }
        Err(unavailable())
    }
}

fn list_refs(repo: &Path, revision: &str) -> Result<Vec<(String, String)>, ClewError> {
    // The compiler uses its own home/configuration. A local sort override
    // would make an otherwise faithful ref list sort differently.
    let local_config = git(
        repo,
        &["config", "--includes", "--name-only", "--list"],
        None,
        128 * 1024,
    )?;
    if local_config.split(|b| *b == b'\n').any(|key| {
        key.eq_ignore_ascii_case(b"versionsort.suffix")
            || key.eq_ignore_ascii_case(b"versionsort.prereleasesuffix")
    }) {
        return Err(ClewError::new(
            ErrorCode::UnsupportedProjectConfiguration,
            "Gradle Git tag input does not support checkout-specific Git version-sort settings.",
        ));
    }
    read_refs(repo, revision.len())
}

fn read_refs(repo: &Path, oid_length: usize) -> Result<Vec<(String, String)>, ClewError> {
    let bytes = git(
        repo,
        &[
            "for-each-ref",
            "--sort=refname",
            &format!("--count={}", MAX_REFS + 1),
            "--format=%(refname)%00%(objectname)%00%(symref)",
            "refs/tags/",
        ],
        None,
        (MAX_REFS + 1) * (MAX_REF_BYTES + 70),
    )?;
    let text = std::str::from_utf8(&bytes).map_err(|_| unavailable())?;
    if text.lines().count() > MAX_REFS {
        return Err(limit());
    }
    text.lines()
        .map(|line| {
            let mut columns = line.split('\0');
            let name = columns.next().ok_or_else(unavailable)?;
            let oid = columns.next().ok_or_else(unavailable)?;
            if columns.next() != Some("")
                || columns.next().is_some()
                || !name.starts_with("refs/tags/")
                || name.len() > MAX_REF_BYTES
                || name.chars().any(|c| c.is_whitespace() || c.is_control())
                || !valid_oid(oid, oid_length)
            {
                return Err(unavailable());
            }
            Ok((name.to_owned(), oid.to_owned()))
        })
        .collect()
}

fn installed_refs(repo: &Path, expected: &[(String, String)]) -> Result<bool, ClewError> {
    let Some((_, oid)) = expected.first() else {
        return Ok(true);
    };
    Ok(read_refs(repo, oid.len())? == expected)
}

fn valid_oid(value: &str, length: usize) -> bool {
    matches!(length, 40 | 64)
        && value.len() == length
        && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn read_object(repo: &Path, oid: &str) -> Result<(String, Vec<u8>), ClewError> {
    let input = format!("{oid}\n");
    let response = git(
        repo,
        &["cat-file", "--batch"],
        Some(input.as_bytes()),
        MAX_OBJECT_BYTES + 128,
    )?;
    let split = response
        .iter()
        .position(|byte| *byte == b'\n')
        .ok_or_else(unavailable)?;
    let header = std::str::from_utf8(&response[..split]).map_err(|_| unavailable())?;
    let fields: Vec<_> = header.split(' ').collect();
    let [returned, kind @ ("tag" | "commit"), size] = fields.as_slice() else {
        return Err(unavailable());
    };
    let size: usize = size.parse().map_err(|_| unavailable())?;
    if size > MAX_OBJECT_BYTES {
        return Err(limit());
    }
    let body = response[split + 1..]
        .strip_suffix(b"\n")
        .ok_or_else(unavailable)?;
    if *returned != oid || size != body.len() {
        return Err(unavailable());
    }
    Ok((kind.to_string(), body.to_vec()))
}

fn git(
    repo: &Path,
    args: &[&str],
    input: Option<&[u8]>,
    limit_bytes: usize,
) -> Result<Vec<u8>, ClewError> {
    let mut child = isolated_git_command(repo)
        .args(args)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(if args[0] == "for-each-ref" {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .spawn()
        .map_err(|_| unavailable())?;
    // Git can exit zero while warning that a broken ref was skipped. Treat
    // any such diagnostic as unavailable authority without retaining its text.
    let diagnostics = child.stderr.take().map(|mut stderr| {
        std::thread::spawn(move || {
            let mut byte = [0];
            matches!(stderr.read(&mut byte), Ok(0))
        })
    });
    let result = (|| {
        if let Some(input) = input {
            child
                .stdin
                .take()
                .ok_or_else(unavailable)?
                .write_all(input)
                .map_err(|_| unavailable())?;
        }
        let mut bytes = Vec::new();
        child
            .stdout
            .take()
            .ok_or_else(unavailable)?
            .take(limit_bytes as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| unavailable())?;
        if bytes.len() > limit_bytes {
            return Err(limit());
        }
        Ok(bytes)
    })();
    if result.is_err() {
        let _ = child.kill();
    }
    let status = child.wait().map_err(|_| unavailable());
    let clean = diagnostics.is_none_or(|thread| matches!(thread.join(), Ok(true)));
    let bytes = result?;
    if !status?.success() || !clean {
        return Err(unavailable());
    }
    Ok(bytes)
}

fn limit() -> ClewError {
    ClewError::new(
        ErrorCode::ResourceLimit,
        "The complete local Gradle Git tag input exceeds the supported bounds.",
    )
}

fn unavailable() -> ClewError {
    ClewError::new(
        ErrorCode::UnsupportedProjectConfiguration,
        "The complete local Gradle Git tag input is unavailable. Use direct local tags that resolve to present commit objects.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(repo: &Path, args: &[&str]) -> Vec<u8> {
        let output = isolated_git_command(repo)
            .args(args)
            .env("GIT_AUTHOR_NAME", "Fixture")
            .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
            .env("GIT_COMMITTER_NAME", "Fixture")
            .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
            .output()
            .unwrap();
        assert!(output.status.success(), "{args:?}: {:?}", output.stderr);
        output.stdout
    }

    fn init(repo: &Path, text: &str) -> String {
        command(
            repo,
            &["init", "--quiet", "--template=", "--object-format=sha1"],
        );
        std::fs::write(repo.join("source.txt"), text).unwrap();
        command(repo, &["add", "--", "source.txt"]);
        command(repo, &["commit", "--quiet", "-m", text]);
        String::from_utf8(command(repo, &["rev-parse", "HEAD"]))
            .unwrap()
            .trim()
            .into()
    }

    #[test]
    fn import_preserves_version_sort_and_tag_objects_without_source_history() {
        let source = tempfile::tempdir().unwrap();
        let sealed = tempfile::tempdir().unwrap();
        let revision = init(source.path(), "owner-only source");
        let generated_head = init(sealed.path(), "filtered code");
        command(source.path(), &["tag", "v2"]);
        command(
            source.path(),
            &["tag", "-a", "v10", "-m", "owned annotation"],
        );
        let tag_inputs = |hash: &str, count| {
            Ok(Binding {
                id: Uuid::from_bytes(hash.as_bytes()[7..23].try_into().unwrap()),
                count,
            })
        };
        let tags = LocalTags::read(source.path(), &revision, tag_inputs).unwrap();
        assert_eq!(tags.binding.count, 2);
        tags.import(sealed.path()).unwrap();
        for arguments in [
            vec!["tag", "--sort=-v:refname"],
            vec!["show-ref", "--tags", "-d"],
            vec!["rev-parse", "v10^{commit}"],
        ] {
            assert_eq!(
                command(source.path(), &arguments),
                command(sealed.path(), &arguments)
            );
        }
        assert_eq!(
            command(sealed.path(), &["rev-parse", "HEAD"]),
            format!("{generated_head}\n").as_bytes()
        );
        let owner_tree =
            String::from_utf8(command(source.path(), &["rev-parse", "HEAD^{tree}"])).unwrap();
        assert!(
            !isolated_git_command(sealed.path())
                .args(["cat-file", "-e", owner_tree.trim()])
                .stderr(Stdio::null())
                .status()
                .unwrap()
                .success()
        );

        command(source.path(), &["tag", "v20"]);
        let changed = LocalTags::read(source.path(), &revision, tag_inputs).unwrap();
        assert_ne!(changed.binding, tags.binding);
        assert_eq!(
            command(source.path(), &["rev-parse", "HEAD"]),
            format!("{revision}\n").as_bytes()
        );
        assert_eq!(
            command(sealed.path(), &["tag", "--sort=-v:refname"]),
            b"v10\nv2\n"
        );
    }

    #[test]
    fn cached_annotation_chain_remains_subject_to_the_complete_depth_bound() {
        let source = tempfile::tempdir().unwrap();
        let sealed = tempfile::tempdir().unwrap();
        let revision = init(source.path(), "owner");
        init(sealed.path(), "isolated");
        let mut target = revision.clone();
        for index in 0..=MAX_TAG_DEPTH {
            let tag = format!("tag-{index}");
            command(
                source.path(),
                &["tag", "-a", &tag, "-m", "owned annotation", &target],
            );
            target = format!("refs/tags/{tag}");
        }
        let tags = LocalTags::read(source.path(), &revision, |_, count| {
            Ok(Binding {
                id: Uuid::new_v4(),
                count,
            })
        })
        .unwrap();
        assert_eq!(
            tags.import(sealed.path()).unwrap_err().code,
            ErrorCode::ResourceLimit
        );
        assert!(command(sealed.path(), &["tag", "--list"]).is_empty());
    }
}
