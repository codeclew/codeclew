use crate::canonical;
use crate::cas::{CasObject, CasStore};
use crate::error::{ClewError, ErrorCode};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::CString;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Component, Path};
use std::process::{Command, Stdio};

#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt, symlink};

pub const SNAPSHOT_SCHEMA: &str = "codeclew-repository-input-snapshot/2.0";
const BLOB_SCHEMA: &str = "codeclew-repository-input-blob/2.0";
const MAX_FILE_BYTES: u64 = 1024 * 1024 * 1024;
const LEGACY_EXCLUDES: [&str; 4] = [
    ":(top,glob,exclude).semantic-thread/**",
    ":(top,exclude).semantic-thread",
    ":(top,glob,exclude)**/.semantic-thread/**",
    ":(top,glob,exclude)**/.semantic-thread",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RepositoryInputSnapshot {
    pub schema: String,
    pub snapshot_id: String,
    pub staged_view_digest: String,
    pub cached_view_digest: String,
    pub untracked_view_digest: String,
    pub index: Vec<IndexEntry>,
    pub worktree: Vec<WorktreeEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IndexEntry {
    pub path: String,
    pub mode: u32,
    pub stage: u8,
    pub git_oid: String,
    pub content: CasObject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorktreeEntry {
    pub path: String,
    pub kind: WorktreeKind,
    pub mode: u32,
    pub content: Option<CasObject>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WorktreeKind {
    Missing,
    Regular,
    Symlink,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GitViews {
    staged: Vec<u8>,
    cached: Vec<u8>,
    untracked: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RawIndexEntry {
    path: String,
    mode: u32,
    stage: u8,
    oid: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RawWorktreeEntry {
    Missing,
    Regular { mode: u32, bytes: Vec<u8> },
    Symlink { mode: u32, target: Vec<u8> },
}

pub fn capture(
    repo: &Path,
    store: &CasStore,
) -> Result<(RepositoryInputSnapshot, CasObject), ClewError> {
    capture_with_hook(repo, store, |_| Ok(()))
}

fn capture_with_hook(
    repo: &Path,
    store: &CasStore,
    mut between_reads: impl FnMut(&str) -> Result<(), ClewError>,
) -> Result<(RepositoryInputSnapshot, CasObject), ClewError> {
    let repo = repo.canonicalize().map_err(io_error)?;
    let before = git_views(&repo)?;
    let raw_index = parse_index(&before.staged)?;
    let cached = parse_paths(&before.cached)?;
    let untracked = parse_paths(&before.untracked)?;
    let staged_blobs = read_git_blobs(&repo, raw_index.iter().map(|entry| entry.oid.as_str()))?;
    let index = raw_index
        .into_iter()
        .map(|entry| {
            let bytes = staged_blobs
                .get(&entry.oid)
                .ok_or_else(|| corrupt_input("staged Git object was not returned by cat-file"))?;
            Ok(IndexEntry {
                path: entry.path,
                mode: entry.mode,
                stage: entry.stage,
                git_oid: entry.oid,
                content: store.put(BLOB_SCHEMA, bytes)?,
            })
        })
        .collect::<Result<Vec<_>, ClewError>>()?;

    let mut worktree_paths = cached.into_iter().chain(untracked).collect::<Vec<_>>();
    worktree_paths.sort();
    worktree_paths.dedup();
    let root = FdRoot::open(&repo)?;
    let mut worktree = Vec::with_capacity(worktree_paths.len());
    for path in worktree_paths {
        let first = root.read(&path)?;
        between_reads(&path)?;
        let second = root.read(&path)?;
        if first != second {
            return Err(mutated("repository input changed while it was captured"));
        }
        let (kind, mode, content) = match first {
            RawWorktreeEntry::Missing => (WorktreeKind::Missing, 0, None),
            RawWorktreeEntry::Regular { mode, bytes } => (
                WorktreeKind::Regular,
                mode,
                Some(store.put(BLOB_SCHEMA, &bytes)?),
            ),
            RawWorktreeEntry::Symlink { mode, target } => (
                WorktreeKind::Symlink,
                mode,
                Some(store.put(BLOB_SCHEMA, &target)?),
            ),
        };
        worktree.push(WorktreeEntry {
            path,
            kind,
            mode,
            content,
        });
    }
    let after = git_views(&repo)?;
    if before != after {
        return Err(mutated(
            "Git repository views changed while the snapshot was captured",
        ));
    }
    let mut snapshot = RepositoryInputSnapshot {
        schema: SNAPSHOT_SCHEMA.into(),
        snapshot_id: String::new(),
        staged_view_digest: canonical::hash_bytes(&before.staged),
        cached_view_digest: canonical::hash_bytes(&before.cached),
        untracked_view_digest: canonical::hash_bytes(&before.untracked),
        index,
        worktree,
    };
    snapshot.snapshot_id = canonical::hash(&snapshot).map_err(internal)?;
    let bytes = canonical::bytes(&snapshot).map_err(internal)?;
    let object = store.put(SNAPSHOT_SCHEMA, &bytes)?;
    Ok((snapshot, object))
}

pub fn materialize(
    snapshot: &RepositoryInputSnapshot,
    store: &CasStore,
    destination: &Path,
) -> Result<(), ClewError> {
    verify_snapshot(snapshot)?;
    if destination.exists() {
        return Err(invalid("snapshot destination already exists"));
    }
    fs::create_dir(destination).map_err(io_error)?;
    set_mode(destination, 0o700)?;
    validate_symlinks(snapshot, store)?;
    for entry in &snapshot.worktree {
        if entry.kind == WorktreeKind::Missing {
            continue;
        }
        let path = destination.join(&entry.path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(io_error)?;
            set_mode(parent, 0o700)?;
        }
        let reference = entry
            .content
            .as_ref()
            .ok_or_else(|| corrupt_input("present snapshot entry has no content"))?;
        let lease = store.read(reference, MAX_FILE_BYTES as usize)?;
        match entry.kind {
            WorktreeKind::Regular => {
                let mut options = OpenOptions::new();
                options.create_new(true).write(true).mode(0o600);
                let mut file = options.open(&path).map_err(io_error)?;
                file.write_all(lease.bytes()).map_err(io_error)?;
                file.sync_all().map_err(io_error)?;
                set_mode(
                    &path,
                    if entry.mode & 0o111 != 0 {
                        0o500
                    } else {
                        0o400
                    },
                )?;
            }
            WorktreeKind::Symlink => {
                let target = std::str::from_utf8(lease.bytes())
                    .map_err(|_| invalid("snapshot symlink target is not UTF-8"))?;
                symlink(target, &path).map_err(io_error)?;
            }
            WorktreeKind::Missing => unreachable!(),
        }
    }
    create_synthetic_git(snapshot, store, destination)?;
    seal_tree(destination)?;
    Ok(())
}

fn create_synthetic_git(
    snapshot: &RepositoryInputSnapshot,
    store: &CasStore,
    destination: &Path,
) -> Result<(), ClewError> {
    if snapshot.index.iter().any(|entry| entry.stage != 0) {
        return Err(invalid("unmerged Git index is unsupported in snapshot v2"));
    }
    let object_format = if snapshot.index.iter().all(|entry| entry.git_oid.len() == 40) {
        "sha1"
    } else if snapshot.index.iter().all(|entry| entry.git_oid.len() == 64) {
        "sha256"
    } else {
        return Err(invalid("snapshot mixes Git object formats"));
    };
    git_command(
        destination,
        &["init", "-q", &format!("--object-format={object_format}")],
        None,
    )?;
    for entry in &snapshot.index {
        if entry.mode == 0o160000 {
            return Err(invalid("Gitlinks are unsupported in snapshot v2"));
        }
        let limit = usize::try_from(entry.content.size).map_err(|_| {
            ClewError::new(ErrorCode::ResourceLimit, "staged blob exceeds host size")
        })?;
        let lease = store.read(&entry.content, limit)?;
        let imported = String::from_utf8(git_command(
            destination,
            &["hash-object", "-w", "--stdin"],
            Some(lease.bytes()),
        )?)
        .map_err(|_| corrupt_input("synthetic Git object identity is not UTF-8"))?;
        if imported.trim() != entry.git_oid {
            return Err(corrupt_input(
                "synthetic Git object identity differs from the snapshot",
            ));
        }
        git_command(
            destination,
            &[
                "update-index",
                "--add",
                "--cacheinfo",
                &format!("{:o}", entry.mode),
                &entry.git_oid,
                &entry.path,
            ],
            None,
        )?;
    }
    let tree = String::from_utf8(git_command(destination, &["write-tree"], None)?)
        .map_err(|_| corrupt_input("synthetic Git tree identity is not UTF-8"))?;
    let commit = git_command_with_identity(
        destination,
        &["commit-tree", tree.trim(), "-m", "Codeclew sealed snapshot"],
    )?;
    let commit = String::from_utf8(commit)
        .map_err(|_| corrupt_input("synthetic Git commit identity is not UTF-8"))?;
    git_command(
        destination,
        &["update-ref", "refs/heads/main", commit.trim()],
        None,
    )?;
    git_command(
        destination,
        &["symbolic-ref", "HEAD", "refs/heads/main"],
        None,
    )?;
    Ok(())
}

fn git_command(
    repo: &Path,
    arguments: &[&str],
    input: Option<&[u8]>,
) -> Result<Vec<u8>, ClewError> {
    let mut command = Command::new("git");
    command
        .args(arguments)
        .current_dir(repo)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = command.spawn().map_err(io_error)?;
    if let Some(bytes) = input {
        child
            .stdin
            .take()
            .ok_or_else(|| internal("synthetic Git stdin is unavailable"))?
            .write_all(bytes)
            .map_err(io_error)?;
    }
    let output = child.wait_with_output().map_err(io_error)?;
    if !output.status.success() {
        return Err(invalid("synthetic Git operation failed"));
    }
    Ok(output.stdout)
}

fn git_command_with_identity(repo: &Path, arguments: &[&str]) -> Result<Vec<u8>, ClewError> {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(repo)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Codeclew")
        .env("GIT_AUTHOR_EMAIL", "noreply@example.invalid")
        .env("GIT_AUTHOR_DATE", "946684800 +0000")
        .env("GIT_COMMITTER_NAME", "Codeclew")
        .env("GIT_COMMITTER_EMAIL", "noreply@example.invalid")
        .env("GIT_COMMITTER_DATE", "946684800 +0000")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .map_err(io_error)?;
    if !output.status.success() {
        return Err(invalid("synthetic Git commit failed"));
    }
    Ok(output.stdout)
}

fn verify_snapshot(snapshot: &RepositoryInputSnapshot) -> Result<(), ClewError> {
    let mut unsigned = snapshot.clone();
    unsigned.snapshot_id.clear();
    if snapshot.schema != SNAPSHOT_SCHEMA
        || snapshot.snapshot_id != canonical::hash(&unsigned).map_err(internal)?
        || !snapshot
            .index
            .windows(2)
            .all(|pair| (&pair[0].path, pair[0].stage) <= (&pair[1].path, pair[1].stage))
        || !snapshot
            .worktree
            .windows(2)
            .all(|pair| pair[0].path < pair[1].path)
    {
        return Err(corrupt_input("repository snapshot authority is invalid"));
    }
    for path in snapshot
        .index
        .iter()
        .map(|entry| entry.path.as_str())
        .chain(snapshot.worktree.iter().map(|entry| entry.path.as_str()))
    {
        validate_path(path)?;
    }
    Ok(())
}

fn git_views(repo: &Path) -> Result<GitViews, ClewError> {
    Ok(GitViews {
        staged: git_ls_files(repo, &["--stage"])?,
        cached: git_ls_files(repo, &["--cached"])?,
        untracked: git_ls_files(repo, &["--others", "--exclude-standard"])?,
    })
}

fn git_ls_files(repo: &Path, options: &[&str]) -> Result<Vec<u8>, ClewError> {
    let mut command = Command::new("git");
    command.arg("ls-files").args(options).arg("-z").arg("--");
    command.args(LEGACY_EXCLUDES);
    let output = command
        .current_dir(repo)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null())
        .output()
        .map_err(io_error)?;
    if !output.status.success() {
        return Err(invalid("filtered Git repository view is unavailable"));
    }
    Ok(output.stdout)
}

fn parse_index(bytes: &[u8]) -> Result<Vec<RawIndexEntry>, ClewError> {
    let mut entries = bytes
        .split(|byte| *byte == 0)
        .filter(|row| !row.is_empty())
        .map(|row| {
            let tab = row
                .iter()
                .position(|byte| *byte == b'\t')
                .ok_or_else(|| invalid("Git index row has no path separator"))?;
            let header = std::str::from_utf8(&row[..tab])
                .map_err(|_| invalid("Git index header is not UTF-8"))?;
            let mut fields = header.split(' ');
            let mode = u32::from_str_radix(fields.next().unwrap_or(""), 8)
                .map_err(|_| invalid("Git index mode is invalid"))?;
            let oid = fields.next().unwrap_or("").to_owned();
            validate_oid(&oid)?;
            let stage = fields
                .next()
                .unwrap_or("")
                .parse::<u8>()
                .map_err(|_| invalid("Git index stage is invalid"))?;
            if stage > 3 || fields.next().is_some() {
                return Err(invalid("Git index header has unsupported fields"));
            }
            let path = std::str::from_utf8(&row[tab + 1..])
                .map_err(|_| invalid("repository path is not UTF-8"))?
                .to_owned();
            validate_path(&path)?;
            Ok(RawIndexEntry {
                path,
                mode,
                stage,
                oid,
            })
        })
        .collect::<Result<Vec<_>, ClewError>>()?;
    entries.sort_by(|left, right| (&left.path, left.stage).cmp(&(&right.path, right.stage)));
    Ok(entries)
}

fn parse_paths(bytes: &[u8]) -> Result<Vec<String>, ClewError> {
    let mut paths = bytes
        .split(|byte| *byte == 0)
        .filter(|row| !row.is_empty())
        .map(|row| {
            let path = std::str::from_utf8(row)
                .map_err(|_| invalid("repository path is not UTF-8"))?
                .to_owned();
            validate_path(&path)?;
            Ok(path)
        })
        .collect::<Result<Vec<_>, ClewError>>()?;
    paths.sort();
    paths.dedup();
    Ok(paths)
}

fn validate_path(path: &str) -> Result<(), ClewError> {
    let value = Path::new(path);
    if path.is_empty()
        || value.is_absolute()
        || value.components().any(|component| {
            matches!(
                component,
                Component::CurDir | Component::ParentDir | Component::RootDir
            ) || component.as_os_str() == ".semantic-thread"
        })
    {
        return Err(invalid(
            "repository path is unsafe or belongs to ignored legacy state",
        ));
    }
    Ok(())
}

fn validate_oid(oid: &str) -> Result<(), ClewError> {
    if !matches!(oid.len(), 40 | 64)
        || !oid
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(invalid("Git object identity is invalid"));
    }
    Ok(())
}

fn read_git_blobs<'a>(
    repo: &Path,
    oids: impl Iterator<Item = &'a str>,
) -> Result<BTreeMap<String, Vec<u8>>, ClewError> {
    let unique = oids.map(str::to_owned).collect::<BTreeSet<_>>();
    let mut child = Command::new("git")
        .args(["cat-file", "--batch"])
        .current_dir(repo)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(io_error)?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| internal("cat-file stdin is unavailable"))?;
    let requests = unique.iter().cloned().collect::<Vec<_>>();
    let writer = std::thread::spawn(move || -> std::io::Result<()> {
        for oid in requests {
            stdin.write_all(oid.as_bytes())?;
            stdin.write_all(b"\n")?;
        }
        Ok(())
    });
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| internal("cat-file stdout is unavailable"))?;
    let mut reader = BufReader::new(stdout);
    let mut blobs = BTreeMap::new();
    for requested in &unique {
        let mut header = String::new();
        reader.read_line(&mut header).map_err(io_error)?;
        let fields = header.trim_end().split(' ').collect::<Vec<_>>();
        if fields.len() != 3 || fields[0] != requested || fields[1] != "blob" {
            return Err(corrupt_input("cat-file returned an unexpected Git object"));
        }
        let size = fields[2]
            .parse::<u64>()
            .map_err(|_| corrupt_input("cat-file returned an invalid blob size"))?;
        if size > MAX_FILE_BYTES || size > usize::MAX as u64 {
            return Err(ClewError::new(
                ErrorCode::ResourceLimit,
                "Git blob exceeds snapshot limit",
            ));
        }
        let mut bytes = vec![0; size as usize];
        reader.read_exact(&mut bytes).map_err(io_error)?;
        let mut newline = [0];
        reader.read_exact(&mut newline).map_err(io_error)?;
        if newline != [b'\n'] {
            return Err(corrupt_input("cat-file blob framing is invalid"));
        }
        blobs.insert(requested.clone(), bytes);
    }
    writer
        .join()
        .map_err(|_| internal("cat-file request writer panicked"))?
        .map_err(io_error)?;
    if !child.wait().map_err(io_error)?.success() {
        return Err(corrupt_input("cat-file batch failed"));
    }
    Ok(blobs)
}

struct FdRoot {
    descriptor: OwnedFd,
}

impl FdRoot {
    fn open(repo: &Path) -> Result<Self, ClewError> {
        let path = CString::new(repo.as_os_str().as_bytes())
            .map_err(|_| invalid("repository path contains NUL"))?;
        let descriptor = unsafe {
            libc::open(
                path.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        if descriptor < 0 {
            return Err(io_error(std::io::Error::last_os_error()));
        }
        Ok(Self {
            descriptor: unsafe { OwnedFd::from_raw_fd(descriptor) },
        })
    }

    fn read(&self, relative: &str) -> Result<RawWorktreeEntry, ClewError> {
        validate_path(relative)?;
        let components = relative.split('/').collect::<Vec<_>>();
        let mut parents = Vec::<OwnedFd>::new();
        let mut parent_fd = self.descriptor.as_raw_fd();
        for component in &components[..components.len() - 1] {
            let name = CString::new(component.as_bytes())
                .map_err(|_| invalid("repository path contains NUL"))?;
            let descriptor = unsafe {
                libc::openat(
                    parent_fd,
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
                )
            };
            if descriptor < 0 {
                let error = std::io::Error::last_os_error();
                if error.kind() == std::io::ErrorKind::NotFound {
                    return Ok(RawWorktreeEntry::Missing);
                }
                return Err(io_error(error));
            }
            parents.push(unsafe { OwnedFd::from_raw_fd(descriptor) });
            parent_fd = parents.last().expect("owned descriptor").as_raw_fd();
        }
        let name = CString::new(
            components
                .last()
                .expect("validated non-empty path")
                .as_bytes(),
        )
        .map_err(|_| invalid("repository path contains NUL"))?;
        let mut metadata = std::mem::MaybeUninit::<libc::stat>::uninit();
        let status = unsafe {
            libc::fstatat(
                parent_fd,
                name.as_ptr(),
                metadata.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        if status != 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::NotFound {
                return Ok(RawWorktreeEntry::Missing);
            }
            return Err(io_error(error));
        }
        let metadata = unsafe { metadata.assume_init() };
        let mode = metadata.st_mode;
        match mode & libc::S_IFMT {
            libc::S_IFREG => {
                if metadata.st_size < 0 || metadata.st_size as u64 > MAX_FILE_BYTES {
                    return Err(ClewError::new(
                        ErrorCode::ResourceLimit,
                        "repository file exceeds snapshot limit",
                    ));
                }
                let descriptor = unsafe {
                    libc::openat(
                        parent_fd,
                        name.as_ptr(),
                        libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
                    )
                };
                if descriptor < 0 {
                    return Err(io_error(std::io::Error::last_os_error()));
                }
                let file = unsafe { File::from_raw_fd(descriptor) };
                let mut bytes = Vec::with_capacity(metadata.st_size as usize);
                file.take(MAX_FILE_BYTES + 1)
                    .read_to_end(&mut bytes)
                    .map_err(io_error)?;
                if bytes.len() as u64 != metadata.st_size as u64 {
                    return Err(mutated("repository file size changed during capture"));
                }
                Ok(RawWorktreeEntry::Regular {
                    mode: u32::from(mode) & 0o777,
                    bytes,
                })
            }
            libc::S_IFLNK => {
                let capacity = usize::try_from(metadata.st_size.max(0))
                    .unwrap_or(0)
                    .saturating_add(1)
                    .max(256);
                if capacity as u64 > MAX_FILE_BYTES {
                    return Err(ClewError::new(
                        ErrorCode::ResourceLimit,
                        "symlink target exceeds snapshot limit",
                    ));
                }
                let mut target = vec![0u8; capacity];
                let length = unsafe {
                    libc::readlinkat(
                        parent_fd,
                        name.as_ptr(),
                        target.as_mut_ptr().cast(),
                        target.len(),
                    )
                };
                if length < 0 {
                    return Err(io_error(std::io::Error::last_os_error()));
                }
                if length as usize == target.len() {
                    return Err(mutated("symlink target changed during capture"));
                }
                target.truncate(length as usize);
                Ok(RawWorktreeEntry::Symlink {
                    mode: u32::from(mode) & 0o777,
                    target,
                })
            }
            _ => Err(invalid("repository input is not a regular file or symlink")),
        }
    }
}

fn validate_symlinks(
    snapshot: &RepositoryInputSnapshot,
    store: &CasStore,
) -> Result<(), ClewError> {
    let mut captured = BTreeSet::new();
    for entry in snapshot
        .worktree
        .iter()
        .filter(|entry| entry.kind != WorktreeKind::Missing)
    {
        captured.insert(entry.path.clone());
        let mut parent = Path::new(&entry.path).parent();
        while let Some(path) = parent {
            if path.as_os_str().is_empty() {
                break;
            }
            captured.insert(path.to_string_lossy().into_owned());
            parent = path.parent();
        }
    }
    for entry in snapshot
        .worktree
        .iter()
        .filter(|entry| entry.kind == WorktreeKind::Symlink)
    {
        let reference = entry
            .content
            .as_ref()
            .ok_or_else(|| corrupt_input("symlink has no target"))?;
        let lease = store.read(reference, MAX_FILE_BYTES as usize)?;
        let target = std::str::from_utf8(lease.bytes())
            .map_err(|_| invalid("snapshot symlink target is not UTF-8"))?;
        let resolved = resolve_symlink(&entry.path, target)?;
        if !captured.contains(&resolved) {
            return Err(invalid("snapshot symlink target was not captured"));
        }
    }
    Ok(())
}

fn resolve_symlink(path: &str, target: &str) -> Result<String, ClewError> {
    let target = Path::new(target);
    if target.is_absolute() {
        return Err(invalid("absolute snapshot symlink is unsupported"));
    }
    let mut parts = Path::new(path)
        .parent()
        .into_iter()
        .flat_map(Path::components)
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>();
    for component in target.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(value) => parts.push(value.to_string_lossy().into_owned()),
            Component::ParentDir => {
                if parts.pop().is_none() {
                    return Err(invalid("snapshot symlink escapes its root"));
                }
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(invalid("snapshot symlink escapes its root"));
            }
        }
    }
    if parts.is_empty() {
        return Err(invalid("snapshot symlink resolves to its root"));
    }
    Ok(parts.join("/"))
}

fn seal_tree(root: &Path) -> Result<(), ClewError> {
    let mut entries = walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .map_err(internal)?
        .into_iter()
        .map(|entry| (entry.file_type(), entry.into_path()))
        .collect::<Vec<_>>();
    entries.sort_by_key(|(_, path)| std::cmp::Reverse(path.components().count()));
    for (kind, path) in entries {
        if kind.is_dir() {
            set_mode(&path, 0o500)?;
        } else if kind.is_file() {
            let executable =
                fs::metadata(&path).map_err(io_error)?.permissions().mode() & 0o111 != 0;
            set_mode(&path, if executable { 0o500 } else { 0o400 })?;
        } else if !kind.is_symlink() {
            return Err(invalid(
                "synthetic Git snapshot contains an unsupported entry",
            ));
        }
    }
    Ok(())
}

fn set_mode(path: &Path, mode: u32) -> Result<(), ClewError> {
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).map_err(io_error)
}

fn invalid(message: &str) -> ClewError {
    ClewError::new(ErrorCode::InvalidInput, message)
}

fn corrupt_input(message: &str) -> ClewError {
    ClewError::new(ErrorCode::StateCorrupt, message)
}

fn mutated(message: &str) -> ClewError {
    ClewError::new(ErrorCode::InputMutated, message)
}

fn internal(error: impl std::fmt::Display) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}

fn io_error(error: std::io::Error) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::StateAuthority;

    fn fixture() -> (tempfile::TempDir, tempfile::TempDir, CasStore) {
        let repo = tempfile::tempdir().unwrap();
        Command::new("git")
            .args(["init", "-q"])
            .current_dir(repo.path())
            .status()
            .unwrap();
        Command::new("git")
            .args(["config", "user.email", "test@example.invalid"])
            .current_dir(repo.path())
            .status()
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", "Codeclew Test"])
            .current_dir(repo.path())
            .status()
            .unwrap();
        fs::create_dir(repo.path().join("src")).unwrap();
        fs::write(repo.path().join("src/main.zeta"), b"stable\n").unwrap();
        Command::new("git")
            .args(["add", "."])
            .current_dir(repo.path())
            .status()
            .unwrap();
        Command::new("git")
            .args(["commit", "-qm", "base"])
            .current_dir(repo.path())
            .status()
            .unwrap();
        let state = tempfile::tempdir().unwrap();
        let authority = StateAuthority::open(state.path().join("v2")).unwrap();
        let store = CasStore::open(&authority).unwrap();
        (repo, state, store)
    }

    #[test]
    fn root_and_nested_legacy_state_cannot_change_snapshot_identity() {
        let (repo, _state, store) = fixture();
        let (before, before_object) = capture(repo.path(), &store).unwrap();
        fs::create_dir(repo.path().join(".semantic-thread")).unwrap();
        fs::write(repo.path().join(".semantic-thread/private"), b"secret-a").unwrap();
        fs::create_dir_all(repo.path().join("src/.semantic-thread")).unwrap();
        fs::write(
            repo.path().join("src/.semantic-thread/private"),
            b"secret-b",
        )
        .unwrap();
        Command::new("git")
            .args([
                "add",
                "-f",
                ".semantic-thread/private",
                "src/.semantic-thread/private",
            ])
            .current_dir(repo.path())
            .status()
            .unwrap();
        set_mode(&repo.path().join(".semantic-thread"), 0o000).unwrap();
        set_mode(&repo.path().join("src/.semantic-thread"), 0o000).unwrap();
        let captured = capture(repo.path(), &store);
        set_mode(&repo.path().join(".semantic-thread"), 0o700).unwrap();
        set_mode(&repo.path().join("src/.semantic-thread"), 0o700).unwrap();
        let (after, after_object) = captured.unwrap();
        assert_eq!(before, after);
        assert_eq!(before_object, after_object);
    }

    #[test]
    fn staged_and_worktree_bytes_are_distinct_authorities() {
        let (repo, _state, store) = fixture();
        let path = repo.path().join("src/main.zeta");
        fs::write(&path, b"staged\n").unwrap();
        Command::new("git")
            .args(["add", "src/main.zeta"])
            .current_dir(repo.path())
            .status()
            .unwrap();
        fs::write(&path, b"worktree\n").unwrap();
        let (snapshot, _) = capture(repo.path(), &store).unwrap();
        let staged = store.read(&snapshot.index[0].content, 1024).unwrap();
        let working = store
            .read(snapshot.worktree[0].content.as_ref().unwrap(), 1024)
            .unwrap();
        assert_eq!(staged.bytes(), b"staged\n");
        assert_eq!(working.bytes(), b"worktree\n");
    }

    #[test]
    fn concurrent_file_mutation_is_typed_and_never_published() {
        let (repo, _state, store) = fixture();
        let path = repo.path().join("src/main.zeta");
        let error = capture_with_hook(repo.path(), &store, |relative| {
            if relative == "src/main.zeta" {
                fs::write(&path, b"changed\n").map_err(io_error)?;
            }
            Ok(())
        })
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::InputMutated);
    }

    #[test]
    fn materialization_is_sealed_and_rejects_escaping_symlink() {
        let (repo, state, store) = fixture();
        symlink("../../outside", repo.path().join("src/escape")).unwrap();
        Command::new("git")
            .args(["add", "src/escape"])
            .current_dir(repo.path())
            .status()
            .unwrap();
        let (snapshot, _) = capture(repo.path(), &store).unwrap();
        let destination = state.path().join("materialized");
        let error = materialize(&snapshot, &store, &destination).unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidInput);
    }

    #[test]
    fn safe_snapshot_materializes_as_read_only_private_tree() {
        let (repo, state, store) = fixture();
        symlink("main.zeta", repo.path().join("src/current.zeta")).unwrap();
        Command::new("git")
            .args(["add", "src/current.zeta"])
            .current_dir(repo.path())
            .status()
            .unwrap();
        let (snapshot, _) = capture(repo.path(), &store).unwrap();
        let destination = state.path().join("materialized-safe");
        materialize(&snapshot, &store, &destination).unwrap();
        let second_destination = state.path().join("materialized-safe-second");
        materialize(&snapshot, &store, &second_destination).unwrap();
        assert_eq!(
            fs::read(destination.join("src/main.zeta")).unwrap(),
            b"stable\n"
        );
        assert_eq!(
            fs::read_link(destination.join("src/current.zeta")).unwrap(),
            Path::new("main.zeta")
        );
        assert_eq!(
            fs::metadata(&destination).unwrap().permissions().mode() & 0o777,
            0o500
        );
        assert_eq!(
            fs::metadata(destination.join("src/main.zeta"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o400
        );
        let revision = |path: &Path| {
            String::from_utf8(
                Command::new("git")
                    .args(["rev-parse", "HEAD"])
                    .current_dir(path)
                    .env("GIT_OPTIONAL_LOCKS", "0")
                    .output()
                    .unwrap()
                    .stdout,
            )
            .unwrap()
        };
        assert_eq!(revision(&destination), revision(&second_destination));
        assert_eq!(
            fs::metadata(destination.join(".git/config"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o400
        );
    }
}
