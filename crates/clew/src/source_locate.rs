use crate::canonical;
use crate::error::{ClewError, ErrorCode};
use crate::repository_snapshot;
use crate::state;
use crate::text_authority;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read};
use std::path::{Component, Path};
use std::process::Stdio;

pub const SOURCE_LOCATE_REQUEST_SCHEMA: &str = "codeclew-source-locate-request/1.0";
pub const SOURCE_LOCATE_RESULT_SCHEMA: &str = "codeclew-source-locate-result/1.0";
pub const DIRECT_SOURCE_LOCATE_RESULT_SCHEMA: &str = "codeclew-source-locate-result/1.1";
const DIRECT_AUTHORITY_SCHEMA: &str = "codeclew-direct-git-source-authority/1.0";
const MAX_REQUEST_BYTES: usize = 1024 * 1024;
const MAX_PATHS: usize = 1000;
const MAX_PATH_BYTES: usize = 4096;
const MAX_LITERAL_BYTES: usize = 512;
const MAX_MATCHES: usize = 64;
const MAX_FILE_BYTES: usize = 16 * 1024 * 1024;
const MAX_TOTAL_BYTES: usize = 16 * 1024 * 1024;
const MAX_RESULT_BYTES: usize = 64 * 1024;
const MAX_TREE_ENTRIES: usize = 200_000;
const MAX_TREE_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceLocateRequest {
    pub schema: String,
    pub literal: String,
    pub paths: Vec<String>,
    pub max_matches: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceLocateAuthority {
    pub session_id: String,
    pub session_authority_digest: String,
    pub context_id: String,
    pub context_evidence_digest: String,
    pub repository_key: String,
    pub base_revision: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct SourceMatch {
    path: String,
    byte_start: usize,
    byte_end: usize,
    content_digest: String,
}

pub fn max_request_bytes() -> usize {
    MAX_REQUEST_BYTES
}

pub fn parse_request(source: &[u8]) -> Result<(SourceLocateRequest, String), ClewError> {
    if source.len() > MAX_REQUEST_BYTES {
        return Err(invalid("source locate request exceeds 1 MiB"));
    }
    let value = canonical::parse_json_strict(source)
        .map_err(|error| invalid(&format!("source locate request JSON is invalid: {error}")))?;
    let request: SourceLocateRequest = serde_json::from_value(value)
        .map_err(|error| invalid(&format!("source locate request shape is invalid: {error}")))?;
    validate_request(&request)?;
    let digest = canonical::hash(&request).map_err(internal)?;
    Ok((request, digest))
}

pub fn locate(
    root: &Path,
    authority: SourceLocateAuthority,
    request: &SourceLocateRequest,
    request_digest: &str,
) -> Result<serde_json::Value, ClewError> {
    validate_request(request)?;
    if !canonical_digest(request_digest) {
        return Err(invalid("source locate request digest is invalid"));
    }
    let root = root.canonicalize().map_err(io_error)?;
    if !root.is_dir() {
        return Err(invalid("source locate root is not a directory"));
    }

    let mut contents = Vec::with_capacity(request.paths.len());
    let mut scanned_bytes = 0usize;
    for relative in &request.paths {
        let path = verified_regular_file(&root, relative)?;
        let metadata = fs::metadata(&path).map_err(io_error)?;
        let file_bytes = usize::try_from(metadata.len())
            .map_err(|_| resource("source locate file exceeds the host size"))?;
        if file_bytes > MAX_FILE_BYTES {
            return Err(resource("source locate file exceeds 16 MiB"));
        }
        scanned_bytes = scanned_bytes
            .checked_add(file_bytes)
            .ok_or_else(|| resource("source locate byte count overflow"))?;
        if scanned_bytes > MAX_TOTAL_BYTES {
            return Err(resource("source locate input exceeds 16 MiB"));
        }
        let content = read_exact_regular_file(&path, file_bytes)?;
        contents.push((relative.clone(), content));
    }

    let source_snapshot_digest = canonical::hash(&serde_json::json!({
        "sessionAuthorityDigest":&authority.session_authority_digest,
        "repositoryKey":&authority.repository_key,
        "baseRevision":&authority.base_revision,
    }))
    .map_err(internal)?;
    locate_contents(
        SOURCE_LOCATE_RESULT_SCHEMA,
        serde_json::to_value(authority).map_err(|error| internal(error.into()))?,
        source_snapshot_digest,
        request,
        request_digest,
        contents,
    )
}

/// Locates exact bytes directly in a clean repository's pinned Git commit.
/// This path is intentionally independent of session/context and language
/// adapter admission; it grants lexical source authority only.
pub fn locate_direct(
    repo: &Path,
    target_ref: &str,
    request: &SourceLocateRequest,
    request_digest: &str,
) -> Result<serde_json::Value, ClewError> {
    locate_direct_with_hook(repo, target_ref, request, request_digest, |_, _| Ok(()))
}

fn locate_direct_with_hook(
    repo: &Path,
    target_ref: &str,
    request: &SourceLocateRequest,
    request_digest: &str,
    after_authority: impl FnOnce(&Path, &str) -> Result<(), ClewError>,
) -> Result<serde_json::Value, ClewError> {
    validate_request(request)?;
    if !canonical_digest(request_digest) {
        return Err(invalid("source locate request digest is invalid"));
    }
    let repo = canonical_repository(repo)?;
    let qualified_ref = qualify_target_ref(&repo, target_ref)?;
    let revision = verify_direct_authority(&repo, &qualified_ref, None)?;
    let tree_oid = isolated_git_text(
        &repo,
        &["rev-parse", "--verify", &format!("{revision}^{{tree}}")],
    )?;
    if !valid_git_oid(&tree_oid) {
        return Err(invalid("direct source locate Git tree identity is invalid"));
    }
    after_authority(&repo, &revision)?;

    let contents = read_commit_paths(&repo, &revision, &request.paths)?;
    let repository_key = state::repository_key(&repo)?;
    verify_direct_authority(&repo, &qualified_ref, Some(&revision))?;
    let target_ref_digest = canonical::hash_bytes(qualified_ref.as_bytes());
    let unsigned_authority = serde_json::json!({
        "schema":DIRECT_AUTHORITY_SCHEMA,
        "mode":"DIRECT_GIT_COMMIT",
        "repositoryKey":repository_key,
        "baseRevision":revision,
        "treeOid":tree_oid,
        "targetRefDigest":target_ref_digest,
    });
    let authority_digest = canonical::hash(&unsigned_authority).map_err(internal)?;
    let mut authority = unsigned_authority;
    authority["authorityDigest"] = serde_json::Value::String(authority_digest.clone());
    let source_snapshot_digest = canonical::hash(&serde_json::json!({
        "sourceAuthorityDigest":authority_digest,
        "repositoryKey":authority["repositoryKey"],
        "baseRevision":authority["baseRevision"],
        "treeOid":authority["treeOid"],
    }))
    .map_err(internal)?;
    locate_contents(
        DIRECT_SOURCE_LOCATE_RESULT_SCHEMA,
        authority,
        source_snapshot_digest,
        request,
        request_digest,
        contents,
    )
}

fn locate_contents(
    result_schema: &str,
    authority: serde_json::Value,
    source_snapshot_digest: String,
    request: &SourceLocateRequest,
    request_digest: &str,
    contents: Vec<(String, Vec<u8>)>,
) -> Result<serde_json::Value, ClewError> {
    let needle = request.literal.as_bytes();
    let mut observed_match_count = 0usize;
    let mut matches = Vec::with_capacity(request.max_matches);
    let mut scanned_bytes = 0usize;
    for (relative, content) in contents {
        scanned_bytes = scanned_bytes
            .checked_add(content.len())
            .ok_or_else(|| resource("source locate byte count overflow"))?;
        if scanned_bytes > MAX_TOTAL_BYTES {
            return Err(resource("source locate input exceeds 16 MiB"));
        }
        let content_digest = canonical::hash_bytes(&content);
        let mut offset = 0usize;
        while offset <= content.len().saturating_sub(needle.len()) {
            let Some(relative_start) = find_bytes(&content[offset..], needle) else {
                break;
            };
            let byte_start = offset + relative_start;
            let byte_end = byte_start + needle.len();
            observed_match_count = observed_match_count
                .checked_add(1)
                .ok_or_else(|| resource("source locate match count overflow"))?;
            if observed_match_count <= request.max_matches {
                matches.push(SourceMatch {
                    path: relative.clone(),
                    byte_start,
                    byte_end,
                    content_digest: content_digest.clone(),
                });
            }
            offset = byte_end;
        }
    }

    let (status, reason_code, completeness, truncated, visible_matches) =
        if observed_match_count > request.max_matches {
            (
                "TRUNCATED",
                Some("INCOMPLETE_MATCH_LIMIT"),
                "INCOMPLETE_LIMIT",
                true,
                Vec::new(),
            )
        } else {
            ("COMPLETE", None, "QUERY_COMPLETE", false, matches)
        };
    let literal_digest = canonical::hash_bytes(needle);
    let mut result = serde_json::json!({
        "schema":result_schema,
        "status":status,
        "reasonCode":reason_code,
        "completeness":completeness,
        "truncated":truncated,
        "semanticResolution":"NONE",
        "authority":"SNAPSHOT_EXACT_BYTES",
        "sourceSnapshotDigest":source_snapshot_digest,
        "source":authority,
        "requestDigest":request_digest,
        "literalDigest":literal_digest,
        "literalBytes":needle.len(),
        "requestedPathCount":request.paths.len(),
        "scannedPathCount":request.paths.len(),
        "scannedBytes":scanned_bytes,
        "observedMatchCount":observed_match_count,
        "matches":visible_matches,
    });
    if canonical::bytes(&result)
        .map_err(internal)?
        .len()
        .saturating_add(1)
        > MAX_RESULT_BYTES
    {
        result["status"] = serde_json::Value::String("TRUNCATED".into());
        result["reasonCode"] = serde_json::Value::String("INCOMPLETE_OUTPUT_LIMIT".into());
        result["completeness"] = serde_json::Value::String("INCOMPLETE_LIMIT".into());
        result["truncated"] = serde_json::Value::Bool(true);
        result["matches"] = serde_json::json!([]);
    }
    if canonical::bytes(&result)
        .map_err(internal)?
        .len()
        .saturating_add(1)
        > MAX_RESULT_BYTES
    {
        return Err(resource(
            "source locate result exceeds 64 KiB without matches",
        ));
    }
    Ok(result)
}

fn canonical_repository(repo: &Path) -> Result<std::path::PathBuf, ClewError> {
    if !repo.is_absolute()
        || repo
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return Err(invalid(
            "direct source locate repository must be a normalized absolute path",
        ));
    }
    let metadata = fs::symlink_metadata(repo)
        .map_err(|_| invalid("direct source locate repository is unavailable"))?;
    let canonical = repo
        .canonicalize()
        .map_err(|_| invalid("direct source locate repository is unavailable"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() || canonical != repo {
        return Err(invalid(
            "direct source locate repository must be a canonical real directory",
        ));
    }
    let top = isolated_git_text(&canonical, &["rev-parse", "--show-toplevel"])?;
    let top = Path::new(&top)
        .canonicalize()
        .map_err(|_| invalid("direct source locate Git root is unavailable"))?;
    if top != canonical {
        return Err(invalid(
            "direct source locate repository must identify the canonical Git root",
        ));
    }
    Ok(canonical)
}

fn qualify_target_ref(repo: &Path, value: &str) -> Result<String, ClewError> {
    let qualified = if value.starts_with("refs/heads/") {
        value.to_owned()
    } else if !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_/.".contains(&byte))
        && !value.contains("..")
    {
        format!("refs/heads/{value}")
    } else {
        return Err(invalid("direct source locate target ref is invalid"));
    };
    if isolated_git_bytes(repo, &["check-ref-format", &qualified]).is_err() {
        return Err(invalid("direct source locate target ref is invalid"));
    }
    Ok(qualified)
}

fn verify_direct_authority(
    repo: &Path,
    target_ref: &str,
    expected_revision: Option<&str>,
) -> Result<String, ClewError> {
    let status = isolated_git_bytes(
        repo,
        &[
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
            "--ignore-submodules=none",
        ],
    )?;
    if !status.is_empty() {
        return Err(precondition(
            "direct source locate requires a clean Git repository",
        ));
    }
    let head = isolated_git_text(repo, &["rev-parse", "--verify", "HEAD^{commit}"])?;
    let target = isolated_git_text(
        repo,
        &["rev-parse", "--verify", &format!("{target_ref}^{{commit}}")],
    )?;
    if !valid_git_oid(&head) || head != target || expected_revision.is_some_and(|v| v != head) {
        return Err(precondition(
            "direct source locate target ref and HEAD must identify one stable commit",
        ));
    }
    Ok(head)
}

fn read_commit_paths(
    repo: &Path,
    revision: &str,
    paths: &[String],
) -> Result<Vec<(String, Vec<u8>)>, ClewError> {
    let requested = paths
        .iter()
        .map(|path| path.as_bytes().to_vec())
        .collect::<BTreeSet<_>>();
    let mut command = repository_snapshot::isolated_git_command(repo);
    command
        .args(["ls-tree", "-r", "-z", "--full-tree", revision])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = command.spawn().map_err(io_error)?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| internal(anyhow::anyhow!("Git tree stdout is unavailable")))?;
    let selected = parse_selected_tree(BufReader::new(stdout), &requested);
    let selected = match selected {
        Ok(selected) => selected,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    };
    if !child.wait().map_err(io_error)?.success() {
        return Err(invalid("direct source locate commit tree is unavailable"));
    }
    if selected.len() != paths.len() {
        return Err(invalid(
            "direct source locate path is missing from the pinned commit",
        ));
    }
    for entry in selected.values() {
        if !matches!(entry.mode, 0o100644 | 0o100755) || entry.kind != "blob" {
            return Err(invalid(
                "direct source locate paths must identify regular Git blobs",
            ));
        }
    }
    let metadata = repository_snapshot::read_git_blob_metadata(
        repo,
        selected.values().map(|entry| entry.oid.as_str()),
    )?;
    let mut total_bytes = 0usize;
    for entry in selected.values() {
        let (kind, size) = metadata
            .get(&entry.oid)
            .ok_or_else(|| invalid("direct source locate Git blob metadata is unavailable"))?;
        let size = usize::try_from(*size)
            .map_err(|_| resource("source locate file exceeds the host size"))?;
        if kind != "blob" || size > MAX_FILE_BYTES {
            return Err(resource("source locate Git blob exceeds 16 MiB"));
        }
        total_bytes = total_bytes
            .checked_add(size)
            .ok_or_else(|| resource("source locate byte count overflow"))?;
        if total_bytes > MAX_TOTAL_BYTES {
            return Err(resource("source locate input exceeds 16 MiB"));
        }
    }
    let blobs = repository_snapshot::read_git_blobs(
        repo,
        selected.values().map(|entry| entry.oid.as_str()),
    )?;
    paths
        .iter()
        .map(|path| {
            let entry = selected
                .get(path)
                .ok_or_else(|| invalid("direct source locate selected path is unavailable"))?;
            let bytes = blobs
                .get(&entry.oid)
                .ok_or_else(|| invalid("direct source locate selected blob is unavailable"))?;
            Ok((path.clone(), bytes.clone()))
        })
        .collect()
}

#[derive(Debug)]
struct DirectTreeEntry {
    mode: u32,
    kind: String,
    oid: String,
}

fn parse_selected_tree(
    mut reader: impl BufRead,
    requested: &BTreeSet<Vec<u8>>,
) -> Result<BTreeMap<String, DirectTreeEntry>, ClewError> {
    let max_row_bytes = MAX_PATH_BYTES + 128;
    let mut selected = BTreeMap::new();
    let mut entry_count = 0usize;
    let mut tree_bytes = 0usize;
    loop {
        let mut row = Vec::new();
        reader
            .by_ref()
            .take((max_row_bytes + 1) as u64)
            .read_until(0, &mut row)
            .map_err(io_error)?;
        if row.is_empty() {
            break;
        }
        if row.last() != Some(&0) || row.len() > max_row_bytes {
            return Err(resource(
                "direct source locate Git tree row exceeds its budget",
            ));
        }
        entry_count = entry_count
            .checked_add(1)
            .ok_or_else(|| resource("direct source locate Git tree count overflow"))?;
        tree_bytes = tree_bytes
            .checked_add(row.len())
            .ok_or_else(|| resource("direct source locate Git tree byte count overflow"))?;
        if entry_count > MAX_TREE_ENTRIES || tree_bytes > MAX_TREE_BYTES {
            return Err(resource(
                "direct source locate Git tree enumeration exceeds its budget",
            ));
        }
        row.pop();
        let tab = row
            .iter()
            .position(|byte| *byte == b'\t')
            .ok_or_else(|| invalid("direct source locate Git tree row is invalid"))?;
        let raw_path = &row[tab + 1..];
        if !requested.contains(raw_path) {
            continue;
        }
        let path = std::str::from_utf8(raw_path)
            .map_err(|_| invalid("direct source locate selected path is not UTF-8"))?
            .to_owned();
        let header = std::str::from_utf8(&row[..tab])
            .map_err(|_| invalid("direct source locate Git tree header is invalid"))?;
        let mut fields = header.split(' ');
        let mode = u32::from_str_radix(fields.next().unwrap_or(""), 8)
            .map_err(|_| invalid("direct source locate Git mode is invalid"))?;
        let kind = fields.next().unwrap_or("").to_owned();
        let oid = fields.next().unwrap_or("").to_owned();
        if fields.next().is_some() || !valid_git_oid(&oid) || selected.contains_key(&path) {
            return Err(invalid(
                "direct source locate Git tree authority is invalid",
            ));
        }
        selected.insert(path, DirectTreeEntry { mode, kind, oid });
    }
    Ok(selected)
}

fn isolated_git_bytes(repo: &Path, arguments: &[&str]) -> Result<Vec<u8>, ClewError> {
    let output = repository_snapshot::isolated_git_command(repo)
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .map_err(io_error)?;
    if !output.status.success() {
        return Err(invalid("direct source locate Git authority is unavailable"));
    }
    Ok(output.stdout)
}

fn isolated_git_text(repo: &Path, arguments: &[&str]) -> Result<String, ClewError> {
    String::from_utf8(isolated_git_bytes(repo, arguments)?)
        .map(|value| value.trim().to_owned())
        .map_err(|_| invalid("direct source locate Git authority is not UTF-8"))
}

fn valid_git_oid(value: &str) -> bool {
    matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn validate_request(request: &SourceLocateRequest) -> Result<(), ClewError> {
    if request.schema != SOURCE_LOCATE_REQUEST_SCHEMA {
        return Err(invalid("unsupported source locate request schema"));
    }
    if request.literal.is_empty()
        || request.literal.len() > MAX_LITERAL_BYTES
        || request.literal.contains('\0')
        || !text_authority::is_nfc(&request.literal)
    {
        return Err(invalid(
            "source locate literal must be non-empty NFC UTF-8 no larger than 512 bytes",
        ));
    }
    if request.paths.is_empty()
        || request.paths.len() > MAX_PATHS
        || request.max_matches == 0
        || request.max_matches > MAX_MATCHES
    {
        return Err(invalid(
            "source locate request requires 1-1000 paths and maxMatches 1-64",
        ));
    }
    let mut previous: Option<&str> = None;
    for path in &request.paths {
        validate_relative_path(path)?;
        if previous.is_some_and(|value| value >= path.as_str()) {
            return Err(invalid(
                "source locate paths must be sorted and unique by UTF-8 bytes",
            ));
        }
        previous = Some(path);
    }
    Ok(())
}

fn validate_relative_path(value: &str) -> Result<(), ClewError> {
    if value.is_empty()
        || value.len() > MAX_PATH_BYTES
        || value.contains('\0')
        || value.contains('\\')
        || !text_authority::is_nfc(value)
    {
        return Err(invalid("source locate path is invalid"));
    }
    let segments = value.split('/').collect::<Vec<_>>();
    if segments
        .iter()
        .any(|segment| segment.is_empty() || matches!(*segment, "." | ".."))
        || segments.join("/") != value
    {
        return Err(invalid(
            "source locate paths must be normalized repository-relative paths",
        ));
    }
    Ok(())
}

fn verified_regular_file(root: &Path, relative: &str) -> Result<std::path::PathBuf, ClewError> {
    let mut current = root.to_path_buf();
    for component in Path::new(relative).components() {
        let Component::Normal(component) = component else {
            return Err(invalid("source locate path is not normalized"));
        };
        let exact_entry_exists = fs::read_dir(&current)
            .map_err(io_error)?
            .filter_map(Result::ok)
            .any(|entry| entry.file_name() == component);
        if !exact_entry_exists {
            return Err(invalid(
                "source locate path spelling differs from the session snapshot",
            ));
        }
        current.push(component);
        let metadata = fs::symlink_metadata(&current)
            .map_err(|_| invalid("source locate path is missing from the session snapshot"))?;
        if metadata.file_type().is_symlink() {
            return Err(invalid("source locate paths may not traverse symlinks"));
        }
    }
    let canonical = current.canonicalize().map_err(io_error)?;
    if !canonical.starts_with(root) || !canonical.is_file() {
        return Err(invalid(
            "source locate path escapes the session snapshot or is not a regular file",
        ));
    }
    Ok(canonical)
}

fn read_exact_regular_file(path: &Path, expected: usize) -> Result<Vec<u8>, ClewError> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    let mut file: File = options.open(path).map_err(io_error)?;
    let before = file.metadata().map_err(io_error)?;
    if !before.is_file() || usize::try_from(before.len()).ok() != Some(expected) {
        return Err(invalid("source locate file changed before reading"));
    }
    let mut bytes = Vec::with_capacity(expected);
    file.by_ref()
        .take(expected.saturating_add(1) as u64)
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    let after = file.metadata().map_err(io_error)?;
    if bytes.len() != expected
        || before.len() != after.len()
        || before.modified().ok() != after.modified().ok()
    {
        return Err(invalid("source locate file changed while reading"));
    }
    Ok(bytes)
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|candidate| candidate == needle)
}

fn canonical_digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

fn invalid(message: &str) -> ClewError {
    ClewError::new(ErrorCode::InvalidInput, message)
}

fn resource(message: &str) -> ClewError {
    ClewError::new(ErrorCode::ResourceLimit, message)
}

fn precondition(message: &str) -> ClewError {
    ClewError::new(ErrorCode::PreconditionFailed, message)
}

fn io_error(error: std::io::Error) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}

fn internal(error: anyhow::Error) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::process::Command;
    use tempfile::TempDir;

    fn request(literal: &str, paths: &[&str], max_matches: usize) -> SourceLocateRequest {
        SourceLocateRequest {
            schema: SOURCE_LOCATE_REQUEST_SCHEMA.into(),
            literal: literal.into(),
            paths: paths.iter().map(|path| (*path).into()).collect(),
            max_matches,
        }
    }

    fn authority() -> SourceLocateAuthority {
        SourceLocateAuthority {
            session_id: "session:test".into(),
            session_authority_digest: format!("sha256:{}", "1".repeat(64)),
            context_id: format!("context:sha256:{}", "3".repeat(64)),
            context_evidence_digest: format!("sha256:{}", "4".repeat(64)),
            repository_key: format!("sha256:{}", "2".repeat(64)),
            base_revision: "0123456789abcdef0123456789abcdef01234567".into(),
        }
    }

    fn git(repo: &Path, arguments: &[&str]) -> String {
        let output = Command::new("git")
            .args(arguments)
            .current_dir(repo)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {arguments:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    fn direct_repository() -> (TempDir, std::path::PathBuf, String) {
        let temporary = TempDir::new().unwrap();
        let repository = temporary.path().join("repository");
        fs::create_dir(&repository).unwrap();
        git(&repository, &["init", "-q", "-b", "main"]);
        fs::write(repository.join("A.kt"), b"before needle before\n").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("A.kt", repository.join("link.kt")).unwrap();
        git(&repository, &["add", "."]);
        git(
            &repository,
            &[
                "-c",
                "user.name=Codeclew Test",
                "-c",
                "user.email=codeclew@localhost",
                "commit",
                "-q",
                "-m",
                "fixture",
            ],
        );
        let repository = repository.canonicalize().unwrap();
        let revision = git(&repository, &["rev-parse", "HEAD"]);
        (temporary, repository, revision)
    }

    #[test]
    fn exact_coordinates_are_sorted_and_non_overlapping() {
        let temporary = TempDir::new().unwrap();
        fs::create_dir_all(temporary.path().join("a")).unwrap();
        fs::write(temporary.path().join("a/A.kt"), b"aaaa\nneedle\n").unwrap();
        fs::write(temporary.path().join("a/B.kt"), b"needle\n").unwrap();
        let query = request("needle", &["a/A.kt", "a/B.kt"], 3);
        let digest = canonical::hash(&query).unwrap();
        let result = locate(temporary.path(), authority(), &query, &digest).unwrap();
        assert_eq!(result["status"], "COMPLETE");
        assert_eq!(result["observedMatchCount"], 2);
        assert_eq!(
            result["matches"],
            json!([
                {"path":"a/A.kt","byteStart":5,"byteEnd":11,"contentDigest":canonical::hash_bytes(b"aaaa\nneedle\n")},
                {"path":"a/B.kt","byteStart":0,"byteEnd":6,"contentDigest":canonical::hash_bytes(b"needle\n")},
            ])
        );

        let overlap = request("aa", &["a/A.kt"], 3);
        let digest = canonical::hash(&overlap).unwrap();
        let result = locate(temporary.path(), authority(), &overlap, &digest).unwrap();
        assert_eq!(result["observedMatchCount"], 2);
        assert_eq!(result["matches"][0]["byteStart"], 0);
        assert_eq!(result["matches"][1]["byteStart"], 2);
    }

    #[test]
    fn overflow_reveals_no_partial_coordinates() {
        let temporary = TempDir::new().unwrap();
        fs::write(temporary.path().join("A.kt"), b"x x x").unwrap();
        let request = request("x", &["A.kt"], 2);
        let digest = canonical::hash(&request).unwrap();
        let result = locate(temporary.path(), authority(), &request, &digest).unwrap();
        assert_eq!(result["status"], "TRUNCATED");
        assert_eq!(result["reasonCode"], "INCOMPLETE_MATCH_LIMIT");
        assert_eq!(result["observedMatchCount"], 3);
        assert_eq!(result["matches"], json!([]));
    }

    #[test]
    fn strict_request_and_snapshot_paths_fail_closed() {
        let duplicate = br#"{"schema":"codeclew-source-locate-request/1.0","literal":"x","literal":"y","paths":["A.kt"],"maxMatches":1}"#;
        assert!(parse_request(duplicate).is_err());
        assert!(validate_request(&request("x", &["../A.kt"], 1)).is_err());
        assert!(validate_request(&request("x", &["a//A.kt"], 1)).is_err());
        assert!(validate_request(&request("x", &["a/A.kt/"], 1)).is_err());
        assert!(validate_request(&request("x", &["B.kt", "A.kt"], 1)).is_err());

        let temporary = TempDir::new().unwrap();
        fs::write(temporary.path().join("A.kt"), b"x").unwrap();
        let case_alias = request("x", &["a.kt"], 1);
        let digest = canonical::hash(&case_alias).unwrap();
        assert!(locate(temporary.path(), authority(), &case_alias, &digest).is_err());
        let missing = request("x", &["missing.kt"], 1);
        let digest = canonical::hash(&missing).unwrap();
        assert!(locate(temporary.path(), authority(), &missing, &digest).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_scope_is_rejected() {
        use std::os::unix::fs::symlink;
        let temporary = TempDir::new().unwrap();
        fs::write(temporary.path().join("target.kt"), b"x").unwrap();
        symlink("target.kt", temporary.path().join("link.kt")).unwrap();
        let request = request("x", &["link.kt"], 1);
        let digest = canonical::hash(&request).unwrap();
        assert!(locate(temporary.path(), authority(), &request, &digest).is_err());
    }

    #[test]
    fn direct_git_locate_binds_pinned_commit_without_private_paths() {
        let (_temporary, repository, revision) = direct_repository();
        let query = request("needle", &["A.kt"], 2);
        let digest = canonical::hash(&query).unwrap();
        let result = locate_direct(&repository, "main", &query, &digest).unwrap();

        assert_eq!(result["schema"], DIRECT_SOURCE_LOCATE_RESULT_SCHEMA);
        assert_eq!(result["source"]["mode"], "DIRECT_GIT_COMMIT");
        assert_eq!(result["source"]["baseRevision"], revision);
        assert!(valid_git_oid(result["source"]["treeOid"].as_str().unwrap()));
        assert_eq!(result["observedMatchCount"], 1);
        assert_eq!(result["matches"][0]["byteStart"], 7);
        assert_eq!(result["matches"][0]["byteEnd"], 13);
        assert!(canonical_digest(
            result["source"]["authorityDigest"].as_str().unwrap()
        ));
        assert!(canonical_digest(
            result["source"]["targetRefDigest"].as_str().unwrap()
        ));
        let mut unsigned_authority = result["source"].clone();
        let authority_digest = unsigned_authority
            .as_object_mut()
            .unwrap()
            .remove("authorityDigest")
            .unwrap();
        assert_eq!(
            authority_digest,
            canonical::hash(&unsigned_authority).unwrap()
        );
        assert_eq!(
            result["sourceSnapshotDigest"],
            canonical::hash(&json!({
                "sourceAuthorityDigest":authority_digest,
                "repositoryKey":result["source"]["repositoryKey"],
                "baseRevision":result["source"]["baseRevision"],
                "treeOid":result["source"]["treeOid"],
            }))
            .unwrap()
        );
        let serialized = canonical::bytes(&result).unwrap();
        assert!(
            !serialized
                .windows(repository.as_os_str().len())
                .any(|window| window == repository.as_os_str().as_encoded_bytes())
        );
        assert!(
            !String::from_utf8(serialized)
                .unwrap()
                .contains("refs/heads/main")
        );
    }

    #[test]
    fn direct_git_locate_rejects_wrong_ref_missing_path_and_symlink() {
        let (_temporary, repository, _revision) = direct_repository();
        let query = request("needle", &["A.kt"], 2);
        let digest = canonical::hash(&query).unwrap();
        assert!(locate_direct(&repository, "missing", &query, &digest).is_err());

        let missing = request("needle", &["missing.kt"], 2);
        let digest = canonical::hash(&missing).unwrap();
        assert!(locate_direct(&repository, "main", &missing, &digest).is_err());

        #[cfg(unix)]
        {
            let symlink = request("needle", &["link.kt"], 2);
            let digest = canonical::hash(&symlink).unwrap();
            let error = locate_direct(&repository, "main", &symlink, &digest).unwrap_err();
            assert!(error.message.contains("regular Git blobs"));
        }

        let nested = repository.join("module");
        fs::create_dir(&nested).unwrap();
        git(&nested, &["init", "-q", "-b", "main"]);
        fs::write(nested.join("nested.txt"), b"needle\n").unwrap();
        git(&nested, &["add", "."]);
        git(
            &nested,
            &[
                "-c",
                "user.name=Codeclew Test",
                "-c",
                "user.email=codeclew@localhost",
                "commit",
                "-q",
                "-m",
                "nested fixture",
            ],
        );
        let nested_revision = git(&nested, &["rev-parse", "HEAD"]);
        git(
            &repository,
            &[
                "update-index",
                "--add",
                "--cacheinfo",
                &format!("160000,{nested_revision},module"),
            ],
        );
        git(
            &repository,
            &[
                "-c",
                "user.name=Codeclew Test",
                "-c",
                "user.email=codeclew@localhost",
                "commit",
                "-q",
                "-m",
                "gitlink fixture",
            ],
        );
        let gitlink = request("needle", &["module"], 2);
        let digest = canonical::hash(&gitlink).unwrap();
        let error = locate_direct(&repository, "main", &gitlink, &digest).unwrap_err();
        assert!(error.message.contains("regular Git blobs"));
    }

    #[test]
    fn direct_git_locate_rejects_dirty_or_concurrently_mutated_checkout() {
        let (_temporary, repository, revision) = direct_repository();
        let query = request("needle", &["A.kt"], 2);
        let digest = canonical::hash(&query).unwrap();

        fs::write(repository.join("A.kt"), b"live mutation without needle\n").unwrap();
        let pinned = read_commit_paths(&repository, &revision, &["A.kt".into()]).unwrap();
        assert_eq!(pinned[0].1, b"before needle before\n");
        let error = locate_direct(&repository, "main", &query, &digest).unwrap_err();
        assert_eq!(error.code, ErrorCode::PreconditionFailed);

        fs::write(repository.join("A.kt"), b"before needle before\n").unwrap();
        let error = locate_direct_with_hook(&repository, "main", &query, &digest, |root, _| {
            fs::write(root.join("A.kt"), b"raced mutation\n").map_err(io_error)?;
            Ok(())
        })
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::PreconditionFailed);
    }
}
