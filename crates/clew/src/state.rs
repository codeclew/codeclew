use crate::error::{ClewError, ErrorCode};
use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::Component;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

#[cfg(unix)]
use std::ffi::{CStr, CString};
#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
#[cfg(unix)]
use std::os::unix::ffi::OsStringExt;
#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, PermissionsExt};

pub const STATE_SCHEMA: &str = "codeclew-state-authority/2.0";

#[derive(Debug, Clone)]
pub struct StateAuthority {
    root: PathBuf,
    _root_handle: Arc<File>,
}

/// A pinned managed-state directory capability. `path` is diagnostic only;
/// every filesystem operation is resolved relative to `handle`.
#[derive(Debug, Clone)]
pub(crate) struct ManagedDirectory {
    path: PathBuf,
    handle: Arc<File>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ManagedEntryKind {
    File,
    Directory,
}

pub(crate) struct ManagedTemporaryDirectory {
    parent: ManagedDirectory,
    directory: ManagedDirectory,
    name: OsString,
}

#[derive(Debug, Clone)]
pub struct RepositoryState {
    pub key: String,
    pub root: PathBuf,
    pub repository_index: PathBuf,
    pub blobs: PathBuf,
    pub model_cache: PathBuf,
    pub compiler_index: PathBuf,
    pub ledger: PathBuf,
}

impl StateAuthority {
    pub fn process_default() -> Result<Self, ClewError> {
        #[cfg(unix)]
        {
            let value = std::env::var_os("CODECLEW_STATE_ROOT_FD")
                .ok_or_else(|| invalid("state root descriptor is unavailable; use ./clew"))?;
            let descriptor = parse_fd(&value, "state root")?;
            validate_directory_fd(descriptor, "state root")?;
            let duplicate = unsafe { libc::fcntl(descriptor, libc::F_DUPFD_CLOEXEC, 3) };
            if duplicate < 0 {
                return Err(io_error(std::io::Error::last_os_error()));
            }
            let handle = unsafe { File::from_raw_fd(duplicate) };
            Self::from_handle(descriptor_path(descriptor)?, handle)
        }
        #[cfg(not(unix))]
        {
            Err(invalid("state descriptor authority requires POSIX"))
        }
    }

    /// Test-only constructor. Production authority is accepted exclusively as
    /// an inherited descriptor from the verified launcher.
    #[cfg(test)]
    pub fn open(root: PathBuf) -> Result<Self, ClewError> {
        if !root.is_absolute()
            || root
                .components()
                .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
        {
            return Err(invalid("state root must use normalized absolute spelling"));
        }
        #[cfg(unix)]
        {
            let parent = root
                .parent()
                .ok_or_else(|| invalid("state root has no parent"))?
                .canonicalize()
                .map_err(io_error)?;
            let name = root
                .file_name()
                .ok_or_else(|| invalid("state root has no directory name"))?;
            let root = parent.join(name);
            let handle = open_absolute_private_directory(&root, true)?;
            Self::from_handle(root, handle)
        }
        #[cfg(not(unix))]
        {
            Err(invalid("state descriptor authority requires POSIX"))
        }
    }

    fn from_handle(root: PathBuf, handle: File) -> Result<Self, ClewError> {
        let metadata = handle.metadata().map_err(io_error)?;
        if !metadata.is_dir() {
            return Err(invalid("state root descriptor must be a directory"));
        }
        #[cfg(unix)]
        {
            if metadata.uid() != unsafe { libc::geteuid() } {
                return Err(invalid("state root descriptor must be caller-owned"));
            }
            if metadata.permissions().mode() & 0o077 != 0 {
                return Err(invalid("state root descriptor must have mode 0700"));
            }
        }
        let authority = Self {
            root,
            _root_handle: Arc::new(handle),
        };
        for child in [
            "runtimes",
            "repos",
            "sessions",
            "missions",
            "workspaces",
            "threads",
            "runs",
            "locks",
            "tmp",
            "quarantine",
            "objects",
            "objects/sha256",
            "objects/packs-v3",
            "objects/catalog-v1",
            "objects/catalog-v1/snapshots",
            "objects/catalog-v1/records",
            "generations",
            "attempts",
            "gc",
        ] {
            authority.ensure_private_directory(Path::new(child))?;
        }
        Ok(authority)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn objects_root(&self) -> PathBuf {
        self.root.join("objects/sha256")
    }

    pub fn locks_root(&self) -> PathBuf {
        self.root.join("locks")
    }

    pub fn quarantine_root(&self) -> PathBuf {
        self.root.join("quarantine")
    }

    pub fn attempts_root(&self) -> PathBuf {
        self.root.join("attempts")
    }

    pub(crate) fn directory(&self, relative: &Path) -> Result<ManagedDirectory, ClewError> {
        let handle = self.open_private_directory(relative, true)?;
        Ok(ManagedDirectory {
            path: self.root.join(relative),
            handle: Arc::new(handle),
        })
    }

    pub(crate) fn directory_at(&self, path: &Path) -> Result<ManagedDirectory, ClewError> {
        let relative = self.relative_path(path)?;
        self.directory(relative)
    }

    pub fn private_file_exists(&self, path: &Path) -> Result<bool, ClewError> {
        let relative = self.relative_path(path)?;
        let (parent, name) = split_relative_file(relative)?;
        self.directory(parent)?.file_exists(name)
    }

    pub fn read_private_file(&self, path: &Path, max_bytes: usize) -> Result<Vec<u8>, ClewError> {
        let relative = self.relative_path(path)?;
        let (parent, name) = split_relative_file(relative)?;
        self.directory(parent)?.read_file(name, max_bytes)
    }

    pub fn open_private_append(&self, path: &Path) -> Result<File, ClewError> {
        let relative = self.relative_path(path)?;
        let (parent, name) = split_relative_file(relative)?;
        self.directory(parent)?.open_append(name)
    }

    pub fn repository(&self, repo: &Path) -> Result<RepositoryState, ClewError> {
        let canonical = repo.canonicalize().map_err(io_error)?;
        let key = repository_key(&canonical)?;
        let relative_root = Path::new("repos").join(&key);
        let root = self.root.join(&relative_root);
        let repository_index = root.join("repository-index");
        let blobs = root.join("blobs/sha256");
        let model_cache = root.join("model-cache");
        let compiler_index = root.join("compiler-index");
        for directory in [
            relative_root.clone(),
            relative_root.join("repository-index"),
            relative_root.join("blobs/sha256"),
            relative_root.join("model-cache"),
            relative_root.join("compiler-index"),
        ] {
            self.ensure_private_directory(&directory)?;
        }
        Ok(RepositoryState {
            key,
            ledger: root.join("ledger.sqlite3"),
            root,
            repository_index,
            blobs,
            model_cache,
            compiler_index,
        })
    }

    pub fn create_private_file(&self, relative: &Path) -> Result<File, ClewError> {
        let (parent, name) = split_relative_file(relative)?;
        let directory = self.open_private_directory(parent, true)?;
        open_new_private_file_at(&directory, name)
    }

    pub fn session_root(&self, session_id: &str) -> Result<PathBuf, ClewError> {
        let name = managed_id_component(session_id, "session:")?;
        let relative = Path::new("sessions").join(name);
        self.ensure_private_directory(&relative)?;
        let path = self.root.join(relative);
        Ok(path)
    }

    pub fn mission_root(&self, mission_id: &str) -> Result<PathBuf, ClewError> {
        let name = managed_id_component(mission_id, "mission:")?;
        let relative = Path::new("missions").join(name);
        self.ensure_private_directory(&relative)?;
        Ok(self.root.join(relative))
    }

    pub fn workspace_root(&self, workspace_id: &str) -> Result<PathBuf, ClewError> {
        let name = managed_id_component(workspace_id, "workspace:")?;
        let relative = Path::new("workspaces").join(name);
        self.ensure_private_directory(&relative)?;
        Ok(self.root.join(relative))
    }

    pub fn thread_root(&self, thread_id: &str) -> Result<PathBuf, ClewError> {
        let name = managed_id_component(thread_id, "thread:")?;
        let relative = Path::new("threads").join(name);
        self.ensure_private_directory(&relative)?;
        Ok(self.root.join(relative))
    }

    pub fn run_root(&self, run_id: &str) -> Result<PathBuf, ClewError> {
        let name = managed_id_component(run_id, "run:")?;
        let relative = Path::new("runs").join(name);
        self.ensure_private_directory(&relative)?;
        let path = self.root.join(relative);
        Ok(path)
    }

    pub fn write_private_atomic(&self, path: &Path, bytes: &[u8]) -> Result<(), ClewError> {
        let relative = self.relative_path(path)?;
        let (parent, name) = split_relative_file(relative)?;
        let directory = self.open_private_directory(parent, true)?;
        validate_replace_target_at(&directory, name)?;
        let temporary_name = format!(".tmp-{}", uuid::Uuid::new_v4());
        let temporary_name = std::ffi::OsStr::new(&temporary_name);
        let mut file = open_new_private_file_at(&directory, temporary_name)?;
        file.write_all(bytes).map_err(io_error)?;
        file.sync_all().map_err(io_error)?;
        drop(file);
        if let Err(error) = rename_at(&directory, temporary_name, name) {
            let _ = unlink_at(&directory, temporary_name);
            return Err(error);
        }
        directory.sync_all().map_err(io_error)?;
        Ok(())
    }

    fn relative_path<'a>(&self, path: &'a Path) -> Result<&'a Path, ClewError> {
        let relative = path
            .strip_prefix(&self.root)
            .map_err(|_| invalid("managed state file escapes its authority root"))?;
        validate_relative(relative)?;
        Ok(relative)
    }

    fn ensure_private_directory(&self, relative: &Path) -> Result<(), ClewError> {
        self.open_private_directory(relative, true).map(drop)
    }

    fn open_private_directory(&self, relative: &Path, create: bool) -> Result<File, ClewError> {
        validate_relative_or_empty(relative)?;
        #[cfg(unix)]
        {
            let duplicate =
                unsafe { libc::fcntl(self._root_handle.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 3) };
            if duplicate < 0 {
                return Err(io_error(std::io::Error::last_os_error()));
            }
            let mut directory = unsafe { File::from_raw_fd(duplicate) };
            for component in relative.components() {
                let Component::Normal(name) = component else {
                    return Err(invalid("managed state directory path is not canonical"));
                };
                directory = open_private_child_directory(&directory, name, create)?;
            }
            Ok(directory)
        }
        #[cfg(not(unix))]
        {
            let _ = create;
            Err(invalid("descriptor-relative managed state requires POSIX"))
        }
    }
}

impl ManagedDirectory {
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// Resolve the currently pinned leaf descriptor for an external tool that
    /// only accepts path arguments. This never walks `StateAuthority::root`.
    pub(crate) fn resolved_path(&self) -> Result<PathBuf, ClewError> {
        #[cfg(unix)]
        {
            descriptor_path(self.handle.as_raw_fd())
        }
        #[cfg(not(unix))]
        {
            Err(invalid("managed directory references require POSIX"))
        }
    }

    pub(crate) fn require_path_identity(&self) -> Result<(), ClewError> {
        let path_metadata = fs::symlink_metadata(&self.path).map_err(io_error)?;
        let handle_metadata = self.handle.metadata().map_err(io_error)?;
        #[cfg(unix)]
        if path_metadata.file_type().is_symlink()
            || !path_metadata.is_dir()
            || path_metadata.dev() != handle_metadata.dev()
            || path_metadata.ino() != handle_metadata.ino()
        {
            return Err(invalid(
                "managed directory path no longer names its descriptor authority",
            ));
        }
        Ok(())
    }

    pub(crate) fn identity(&self) -> Result<(u64, u64), ClewError> {
        let metadata = self.handle.metadata().map_err(io_error)?;
        Ok((metadata.dev(), metadata.ino()))
    }

    pub(crate) fn child(&self, relative: &Path) -> Result<Self, ClewError> {
        validate_relative(relative)?;
        let mut directory = duplicate_file(&self.handle)?;
        for component in relative.components() {
            let Component::Normal(name) = component else {
                return Err(invalid("managed state directory path is not canonical"));
            };
            directory = open_private_child_directory(&directory, name, true)?;
        }
        Ok(Self {
            path: self.path.join(relative),
            handle: Arc::new(directory),
        })
    }

    pub(crate) fn existing_child(&self, name: &std::ffi::OsStr) -> Result<Self, ClewError> {
        validate_file_name(name)?;
        let directory = open_private_child_directory(&self.handle, name, false)?;
        Ok(Self {
            path: self.path.join(name),
            handle: Arc::new(directory),
        })
    }

    pub(crate) fn entry_kind(&self, name: &std::ffi::OsStr) -> Result<ManagedEntryKind, ClewError> {
        validate_file_name(name)?;
        let encoded = component_name(name)?;
        let mut status = std::mem::MaybeUninit::<libc::stat>::uninit();
        if unsafe {
            libc::fstatat(
                self.handle.as_raw_fd(),
                encoded.as_ptr(),
                status.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } != 0
        {
            return Err(io_error(std::io::Error::last_os_error()));
        }
        let status = unsafe { status.assume_init() };
        match status.st_mode & libc::S_IFMT {
            libc::S_IFDIR => Ok(ManagedEntryKind::Directory),
            libc::S_IFREG => Ok(ManagedEntryKind::File),
            _ => Err(invalid(
                "managed state entry is not a regular file or directory",
            )),
        }
    }

    pub(crate) fn file_len(&self, name: &std::ffi::OsStr) -> Result<u64, ClewError> {
        let file = self.open_file(name)?;
        Ok(file.metadata().map_err(io_error)?.len())
    }

    pub(crate) fn temporary_child(
        &self,
        prefix: &str,
    ) -> Result<ManagedTemporaryDirectory, ClewError> {
        if prefix.is_empty()
            || prefix.len() > 64
            || !prefix
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            return Err(invalid("managed temporary directory prefix is invalid"));
        }
        let name = OsString::from(format!("{prefix}-{}", uuid::Uuid::new_v4()));
        let handle = create_private_child_directory(&self.handle, &name)?;
        Ok(ManagedTemporaryDirectory {
            parent: self.clone(),
            directory: ManagedDirectory {
                path: self.path.join(&name),
                handle: Arc::new(handle),
            },
            name,
        })
    }

    pub(crate) fn create_file(&self, name: &std::ffi::OsStr) -> Result<File, ClewError> {
        validate_file_name(name)?;
        open_new_private_file_at(&self.handle, name)
    }

    pub(crate) fn open_file(&self, name: &std::ffi::OsStr) -> Result<File, ClewError> {
        validate_file_name(name)?;
        open_existing_private_file_at(&self.handle, name, libc::O_RDONLY)
    }

    pub(crate) fn open_append(&self, name: &std::ffi::OsStr) -> Result<File, ClewError> {
        validate_file_name(name)?;
        open_or_create_private_file_at(&self.handle, name, libc::O_WRONLY | libc::O_APPEND)
    }

    pub(crate) fn open_lock(&self, name: &std::ffi::OsStr) -> Result<File, ClewError> {
        validate_file_name(name)?;
        open_or_create_private_file_at(&self.handle, name, libc::O_RDWR)
    }

    pub(crate) fn file_exists(&self, name: &std::ffi::OsStr) -> Result<bool, ClewError> {
        validate_file_name(name)?;
        private_file_status_at(&self.handle, name).map(|status| status.is_some())
    }

    pub(crate) fn read_file(
        &self,
        name: &std::ffi::OsStr,
        max_bytes: usize,
    ) -> Result<Vec<u8>, ClewError> {
        let file = self.open_file(name)?;
        let metadata = file.metadata().map_err(io_error)?;
        if metadata.len() > max_bytes as u64 {
            return Err(invalid("managed state file exceeds its read bound"));
        }
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        file.take(max_bytes as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(io_error)?;
        if bytes.len() as u64 != metadata.len() {
            return Err(invalid("managed state file changed during read"));
        }
        Ok(bytes)
    }

    pub(crate) fn atomic_write(
        &self,
        name: &std::ffi::OsStr,
        bytes: &[u8],
    ) -> Result<(), ClewError> {
        validate_file_name(name)?;
        validate_replace_target_at(&self.handle, name)?;
        let temporary_name = format!(".tmp-{}", uuid::Uuid::new_v4());
        let temporary_name = std::ffi::OsStr::new(&temporary_name);
        let mut file = self.create_file(temporary_name)?;
        file.write_all(bytes).map_err(io_error)?;
        file.sync_all().map_err(io_error)?;
        drop(file);
        if let Err(error) = rename_at(&self.handle, temporary_name, name) {
            let _ = unlink_at(&self.handle, temporary_name);
            return Err(error);
        }
        self.handle.sync_all().map_err(io_error)
    }

    /// Installs fully durable bytes only when `name` does not yet exist.
    /// A crash can leave an unreferenced `.tmp-*` file, but never a partial
    /// destination. The hard-link step is the create-new commit point.
    pub(crate) fn atomic_create(
        &self,
        name: &std::ffi::OsStr,
        bytes: &[u8],
    ) -> Result<bool, ClewError> {
        validate_file_name(name)?;
        let temporary_name = format!(".tmp-{}", uuid::Uuid::new_v4());
        let temporary_name = std::ffi::OsStr::new(&temporary_name);
        let mut file = self.create_file(temporary_name)?;
        if let Err(error) = file
            .write_all(bytes)
            .and_then(|()| file.sync_all())
            .map_err(io_error)
        {
            drop(file);
            let _ = unlink_at(&self.handle, temporary_name);
            return Err(error);
        }
        drop(file);
        let installed = match link_at_if_absent(&self.handle, temporary_name, name) {
            Ok(installed) => installed,
            Err(error) => {
                let _ = unlink_at(&self.handle, temporary_name);
                return Err(error);
            }
        };
        unlink_at(&self.handle, temporary_name)?;
        self.handle.sync_all().map_err(io_error)?;
        Ok(installed)
    }

    pub(crate) fn rename_to(
        &self,
        source: &std::ffi::OsStr,
        destination: &ManagedDirectory,
        target: &std::ffi::OsStr,
    ) -> Result<(), ClewError> {
        validate_file_name(source)?;
        validate_file_name(target)?;
        rename_between(&self.handle, source, &destination.handle, target)?;
        self.handle.sync_all().map_err(io_error)?;
        if self.handle.as_raw_fd() != destination.handle.as_raw_fd() {
            destination.handle.sync_all().map_err(io_error)?;
        }
        Ok(())
    }

    pub(crate) fn remove_file(&self, name: &std::ffi::OsStr) -> Result<(), ClewError> {
        validate_file_name(name)?;
        unlink_at(&self.handle, name)?;
        self.handle.sync_all().map_err(io_error)
    }

    pub(crate) fn entries(&self) -> Result<Vec<OsString>, ClewError> {
        read_directory_names(&self.handle)
    }
}

impl ManagedTemporaryDirectory {
    pub(crate) fn directory(&self) -> &ManagedDirectory {
        &self.directory
    }
}

impl Drop for ManagedTemporaryDirectory {
    fn drop(&mut self) {
        let _ = remove_directory_tree_at(&self.parent.handle, &self.name);
    }
}

fn validate_relative(path: &Path) -> Result<(), ClewError> {
    if path.as_os_str().is_empty() {
        return Err(invalid("managed state path is empty"));
    }
    validate_relative_or_empty(path)
}

fn validate_relative_or_empty(path: &Path) -> Result<(), ClewError> {
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(invalid("managed state path is not canonical relative"));
    }
    Ok(())
}

fn split_relative_file(path: &Path) -> Result<(&Path, &std::ffi::OsStr), ClewError> {
    validate_relative(path)?;
    let name = path
        .file_name()
        .ok_or_else(|| invalid("managed state file has no name"))?;
    let parent = path.parent().unwrap_or_else(|| Path::new(""));
    Ok((parent, name))
}

fn validate_file_name(name: &std::ffi::OsStr) -> Result<(), ClewError> {
    let path = Path::new(name);
    if name.is_empty()
        || path.is_absolute()
        || path.components().count() != 1
        || !matches!(path.components().next(), Some(Component::Normal(_)))
    {
        return Err(invalid("managed state file name is not canonical"));
    }
    Ok(())
}

#[cfg(unix)]
fn component_name(value: &std::ffi::OsStr) -> Result<CString, ClewError> {
    CString::new(value.as_bytes()).map_err(|_| invalid("managed state path contains NUL"))
}

#[cfg(all(test, unix))]
fn open_absolute_private_directory(path: &Path, create_leaf: bool) -> Result<File, ClewError> {
    if !path.is_absolute() {
        return Err(invalid("state root must be absolute"));
    }
    let components = path
        .components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(Ok(value)),
            Component::RootDir => None,
            _ => Some(Err(invalid("state root path is not canonical"))),
        })
        .collect::<Result<Vec<_>, _>>()?;
    if components.is_empty() {
        return Err(invalid("state root cannot be the filesystem root"));
    }
    let root = CString::new("/").expect("static root path");
    let descriptor = unsafe {
        libc::open(
            root.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if descriptor < 0 {
        return Err(io_error(std::io::Error::last_os_error()));
    }
    let mut directory = unsafe { File::from_raw_fd(descriptor) };
    for (index, component) in components.iter().enumerate() {
        let component = component_name(component)?;
        let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
        let mut child = unsafe { libc::openat(directory.as_raw_fd(), component.as_ptr(), flags) };
        if child < 0 {
            let error = std::io::Error::last_os_error();
            if !create_leaf
                || index + 1 != components.len()
                || error.kind() != std::io::ErrorKind::NotFound
            {
                return Err(io_error(error));
            }
            if unsafe { libc::mkdirat(directory.as_raw_fd(), component.as_ptr(), 0o700) } != 0 {
                let mkdir_error = std::io::Error::last_os_error();
                if mkdir_error.kind() != std::io::ErrorKind::AlreadyExists {
                    return Err(io_error(mkdir_error));
                }
            }
            child = unsafe { libc::openat(directory.as_raw_fd(), component.as_ptr(), flags) };
            if child < 0 {
                return Err(io_error(std::io::Error::last_os_error()));
            }
        }
        directory = unsafe { File::from_raw_fd(child) };
    }
    validate_owned_directory_file(&directory)?;
    if unsafe { libc::fchmod(directory.as_raw_fd(), 0o700) } != 0 {
        return Err(io_error(std::io::Error::last_os_error()));
    }
    validate_private_directory_file(&directory)?;
    Ok(directory)
}

#[cfg(unix)]
fn validate_owned_directory_file(directory: &File) -> Result<(), ClewError> {
    let metadata = directory.metadata().map_err(io_error)?;
    if !metadata.is_dir() || metadata.uid() != unsafe { libc::geteuid() } {
        return Err(invalid(
            "managed state directory type or ownership is unsafe",
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn validate_private_directory_file(directory: &File) -> Result<(), ClewError> {
    validate_owned_directory_file(directory)?;
    if directory.metadata().map_err(io_error)?.permissions().mode() & 0o077 != 0 {
        return Err(invalid("managed state directory permissions are unsafe"));
    }
    Ok(())
}

#[cfg(unix)]
fn open_private_child_directory(
    parent: &File,
    name: &std::ffi::OsStr,
    create: bool,
) -> Result<File, ClewError> {
    let name = component_name(name)?;
    let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    let mut descriptor = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
    if descriptor < 0 {
        let error = std::io::Error::last_os_error();
        if !create || error.kind() != std::io::ErrorKind::NotFound {
            return Err(io_error(error));
        }
        if unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o700) } != 0 {
            let mkdir_error = std::io::Error::last_os_error();
            if mkdir_error.kind() != std::io::ErrorKind::AlreadyExists {
                return Err(io_error(mkdir_error));
            }
        }
        descriptor = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
        if descriptor < 0 {
            return Err(io_error(std::io::Error::last_os_error()));
        }
    }
    let directory = unsafe { File::from_raw_fd(descriptor) };
    validate_owned_directory_file(&directory)?;
    if unsafe { libc::fchmod(directory.as_raw_fd(), 0o700) } != 0 {
        return Err(io_error(std::io::Error::last_os_error()));
    }
    validate_private_directory_file(&directory)?;
    Ok(directory)
}

#[cfg(unix)]
fn create_private_child_directory(
    parent: &File,
    name: &std::ffi::OsStr,
) -> Result<File, ClewError> {
    let name = component_name(name)?;
    if unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o700) } != 0 {
        return Err(io_error(std::io::Error::last_os_error()));
    }
    open_private_child_directory(parent, std::ffi::OsStr::from_bytes(name.to_bytes()), false)
}

#[cfg(not(unix))]
fn create_private_child_directory(
    _parent: &File,
    _name: &std::ffi::OsStr,
) -> Result<File, ClewError> {
    Err(invalid("descriptor-relative managed state requires POSIX"))
}

#[cfg(unix)]
fn duplicate_file(file: &File) -> Result<File, ClewError> {
    let descriptor = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 3) };
    if descriptor < 0 {
        return Err(io_error(std::io::Error::last_os_error()));
    }
    Ok(unsafe { File::from_raw_fd(descriptor) })
}

#[cfg(unix)]
fn open_new_private_file_at(parent: &File, name: &std::ffi::OsStr) -> Result<File, ClewError> {
    let name = component_name(name)?;
    let descriptor = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o600,
        )
    };
    if descriptor < 0 {
        return Err(io_error(std::io::Error::last_os_error()));
    }
    let file = unsafe { File::from_raw_fd(descriptor) };
    let metadata = file.metadata().map_err(io_error)?;
    if !metadata.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err(invalid(
            "managed state file ownership or permissions are unsafe",
        ));
    }
    Ok(file)
}

#[cfg(unix)]
fn open_existing_private_file_at(
    parent: &File,
    name: &std::ffi::OsStr,
    access: i32,
) -> Result<File, ClewError> {
    let name = component_name(name)?;
    let descriptor = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            access | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if descriptor < 0 {
        return Err(io_error(std::io::Error::last_os_error()));
    }
    let file = unsafe { File::from_raw_fd(descriptor) };
    validate_private_file(&file)?;
    Ok(file)
}

#[cfg(unix)]
fn open_or_create_private_file_at(
    parent: &File,
    name: &std::ffi::OsStr,
    access: i32,
) -> Result<File, ClewError> {
    let name = component_name(name)?;
    let common = access | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    let mut attempts = 0_u8;
    let descriptor = loop {
        attempts = attempts.saturating_add(1);
        let existing = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), common) };
        if existing >= 0 {
            break existing;
        }
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::NotFound {
            return Err(io_error(error));
        }
        let created = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                common | libc::O_CREAT | libc::O_EXCL,
                0o600,
            )
        };
        if created >= 0 {
            break created;
        }
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::AlreadyExists {
            return Err(io_error(error));
        }
        if attempts >= 16 {
            return Err(invalid("managed state file changed repeatedly during open"));
        }
    };
    let file = unsafe { File::from_raw_fd(descriptor) };
    validate_private_file(&file)?;
    Ok(file)
}

#[cfg(unix)]
fn validate_private_file(file: &File) -> Result<(), ClewError> {
    let metadata = file.metadata().map_err(io_error)?;
    if !metadata.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err(invalid(
            "managed state file ownership or permissions are unsafe",
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn private_file_status_at(
    parent: &File,
    name: &std::ffi::OsStr,
) -> Result<Option<libc::stat>, ClewError> {
    let name = component_name(name)?;
    let mut status = std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe {
        libc::fstatat(
            parent.as_raw_fd(),
            name.as_ptr(),
            status.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } != 0
    {
        let error = std::io::Error::last_os_error();
        return if error.kind() == std::io::ErrorKind::NotFound {
            Ok(None)
        } else {
            Err(io_error(error))
        };
    }
    let status = unsafe { status.assume_init() };
    if status.st_mode & libc::S_IFMT != libc::S_IFREG
        || status.st_uid != unsafe { libc::geteuid() }
        || status.st_mode & 0o077 != 0
    {
        return Err(invalid("managed state file authority is unsafe"));
    }
    Ok(Some(status))
}

#[cfg(not(unix))]
fn open_new_private_file_at(_parent: &File, _name: &std::ffi::OsStr) -> Result<File, ClewError> {
    Err(invalid("descriptor-relative managed state requires POSIX"))
}

#[cfg(unix)]
fn validate_replace_target_at(parent: &File, name: &std::ffi::OsStr) -> Result<(), ClewError> {
    let name = component_name(name)?;
    let mut status = std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe {
        libc::fstatat(
            parent.as_raw_fd(),
            name.as_ptr(),
            status.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } != 0
    {
        let error = std::io::Error::last_os_error();
        return if error.kind() == std::io::ErrorKind::NotFound {
            Ok(())
        } else {
            Err(io_error(error))
        };
    }
    let status = unsafe { status.assume_init() };
    if status.st_mode & libc::S_IFMT != libc::S_IFREG
        || status.st_uid != unsafe { libc::geteuid() }
        || status.st_mode & 0o077 != 0
    {
        return Err(invalid("managed state replacement target is unsafe"));
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_replace_target_at(_parent: &File, _name: &std::ffi::OsStr) -> Result<(), ClewError> {
    Err(invalid("descriptor-relative managed state requires POSIX"))
}

#[cfg(unix)]
fn rename_at(
    parent: &File,
    source: &std::ffi::OsStr,
    destination: &std::ffi::OsStr,
) -> Result<(), ClewError> {
    let source = component_name(source)?;
    let destination = component_name(destination)?;
    if unsafe {
        libc::renameat(
            parent.as_raw_fd(),
            source.as_ptr(),
            parent.as_raw_fd(),
            destination.as_ptr(),
        )
    } != 0
    {
        return Err(io_error(std::io::Error::last_os_error()));
    }
    Ok(())
}

#[cfg(unix)]
fn rename_between(
    source_parent: &File,
    source: &std::ffi::OsStr,
    destination_parent: &File,
    destination: &std::ffi::OsStr,
) -> Result<(), ClewError> {
    let source = component_name(source)?;
    let destination = component_name(destination)?;
    if unsafe {
        libc::renameat(
            source_parent.as_raw_fd(),
            source.as_ptr(),
            destination_parent.as_raw_fd(),
            destination.as_ptr(),
        )
    } != 0
    {
        return Err(io_error(std::io::Error::last_os_error()));
    }
    Ok(())
}

#[cfg(unix)]
fn link_at_if_absent(
    parent: &File,
    source: &std::ffi::OsStr,
    destination: &std::ffi::OsStr,
) -> Result<bool, ClewError> {
    let source = component_name(source)?;
    let destination = component_name(destination)?;
    if unsafe {
        libc::linkat(
            parent.as_raw_fd(),
            source.as_ptr(),
            parent.as_raw_fd(),
            destination.as_ptr(),
            0,
        )
    } != 0
    {
        let error = std::io::Error::last_os_error();
        return if error.raw_os_error() == Some(libc::EEXIST) {
            Ok(false)
        } else {
            Err(io_error(error))
        };
    }
    Ok(true)
}

#[cfg(not(unix))]
fn link_at_if_absent(
    _parent: &File,
    _source: &std::ffi::OsStr,
    _destination: &std::ffi::OsStr,
) -> Result<bool, ClewError> {
    Err(invalid("descriptor-relative managed state requires POSIX"))
}

#[cfg(not(unix))]
fn rename_at(
    _parent: &File,
    _source: &std::ffi::OsStr,
    _destination: &std::ffi::OsStr,
) -> Result<(), ClewError> {
    Err(invalid("descriptor-relative managed state requires POSIX"))
}

#[cfg(unix)]
fn unlink_at(parent: &File, name: &std::ffi::OsStr) -> Result<(), ClewError> {
    let name = component_name(name)?;
    if unsafe { libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), 0) } != 0 {
        return Err(io_error(std::io::Error::last_os_error()));
    }
    Ok(())
}

#[cfg(unix)]
fn remove_directory_tree_at(parent: &File, name: &std::ffi::OsStr) -> Result<(), ClewError> {
    validate_file_name(name)?;
    let directory = open_private_child_directory(parent, name, false)?;
    remove_directory_contents(&directory)?;
    let name = component_name(name)?;
    if unsafe { libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), libc::AT_REMOVEDIR) } != 0 {
        return Err(io_error(std::io::Error::last_os_error()));
    }
    parent.sync_all().map_err(io_error)
}

#[cfg(unix)]
fn remove_directory_contents(directory: &File) -> Result<(), ClewError> {
    for name in read_directory_names(directory)? {
        let encoded = component_name(&name)?;
        let mut status = std::mem::MaybeUninit::<libc::stat>::uninit();
        if unsafe {
            libc::fstatat(
                directory.as_raw_fd(),
                encoded.as_ptr(),
                status.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } != 0
        {
            return Err(io_error(std::io::Error::last_os_error()));
        }
        let status = unsafe { status.assume_init() };
        if status.st_mode & libc::S_IFMT == libc::S_IFDIR {
            let child = open_private_child_directory(directory, &name, false)?;
            remove_directory_contents(&child)?;
            if unsafe {
                libc::unlinkat(directory.as_raw_fd(), encoded.as_ptr(), libc::AT_REMOVEDIR)
            } != 0
            {
                return Err(io_error(std::io::Error::last_os_error()));
            }
        } else if unsafe { libc::unlinkat(directory.as_raw_fd(), encoded.as_ptr(), 0) } != 0 {
            return Err(io_error(std::io::Error::last_os_error()));
        }
    }
    directory.sync_all().map_err(io_error)
}

#[cfg(not(unix))]
fn remove_directory_tree_at(_parent: &File, _name: &std::ffi::OsStr) -> Result<(), ClewError> {
    Err(invalid("descriptor-relative managed state requires POSIX"))
}

#[cfg(unix)]
fn read_directory_names(directory: &File) -> Result<Vec<OsString>, ClewError> {
    // dup(2) shares the directory stream offset with the pinned authority FD,
    // so a second enumeration would otherwise appear empty. openat(".")
    // creates a fresh open-file description while remaining bound to the same
    // descriptor-authorized directory.
    let current = b".\0";
    let duplicate = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            current.as_ptr().cast(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if duplicate < 0 {
        return Err(io_error(std::io::Error::last_os_error()));
    }
    let stream = unsafe { libc::fdopendir(duplicate) };
    if stream.is_null() {
        unsafe {
            libc::close(duplicate);
        }
        return Err(io_error(std::io::Error::last_os_error()));
    }
    let mut names = Vec::new();
    loop {
        set_errno(0);
        let entry = unsafe { libc::readdir(stream) };
        if entry.is_null() {
            let error = get_errno();
            unsafe {
                libc::closedir(stream);
            }
            if error != 0 {
                return Err(io_error(std::io::Error::from_raw_os_error(error)));
            }
            break;
        }
        let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
        if name != b"." && name != b".." {
            names.push(OsString::from_vec(name.to_vec()));
        }
    }
    names.sort();
    Ok(names)
}

#[cfg(target_os = "linux")]
fn set_errno(value: i32) {
    unsafe { *libc::__errno_location() = value };
}

#[cfg(target_os = "linux")]
fn get_errno() -> i32 {
    unsafe { *libc::__errno_location() }
}

#[cfg(target_os = "macos")]
fn set_errno(value: i32) {
    unsafe { *libc::__error() = value };
}

#[cfg(target_os = "macos")]
fn get_errno() -> i32 {
    unsafe { *libc::__error() }
}

#[cfg(not(unix))]
fn unlink_at(_parent: &File, _name: &std::ffi::OsStr) -> Result<(), ClewError> {
    Err(invalid("descriptor-relative managed state requires POSIX"))
}

#[cfg(unix)]
fn parse_fd(value: &std::ffi::OsStr, label: &str) -> Result<RawFd, ClewError> {
    let text = value
        .to_str()
        .ok_or_else(|| invalid(&format!("{label} descriptor is not UTF-8")))?;
    let descriptor: RawFd = text
        .parse()
        .map_err(|_| invalid(&format!("{label} descriptor is invalid")))?;
    if descriptor < 3 || unsafe { libc::fcntl(descriptor, libc::F_GETFD) } < 0 {
        return Err(invalid(&format!("{label} descriptor is not open")));
    }
    Ok(descriptor)
}

#[cfg(unix)]
fn validate_directory_fd(fd: RawFd, label: &str) -> Result<(), ClewError> {
    let mut status = std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe { libc::fstat(fd, status.as_mut_ptr()) } != 0 {
        return Err(io_error(std::io::Error::last_os_error()));
    }
    let status = unsafe { status.assume_init() };
    if status.st_mode & libc::S_IFMT != libc::S_IFDIR
        || status.st_uid != unsafe { libc::geteuid() }
        || status.st_mode & 0o077 != 0
    {
        return Err(invalid(&format!("{label} descriptor is unsafe")));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn descriptor_path(fd: RawFd) -> Result<PathBuf, ClewError> {
    let path = fs::read_link(format!("/proc/self/fd/{fd}")).map_err(io_error)?;
    if !path.is_absolute() || path.as_os_str().as_bytes().ends_with(b" (deleted)") {
        return Err(invalid("managed directory descriptor has no stable path"));
    }
    Ok(path)
}

#[cfg(target_os = "macos")]
fn descriptor_path(fd: RawFd) -> Result<PathBuf, ClewError> {
    let mut buffer = [0 as libc::c_char; 4096];
    if unsafe { libc::fcntl(fd, libc::F_GETPATH, buffer.as_mut_ptr()) } < 0 {
        return Err(io_error(std::io::Error::last_os_error()));
    }
    let bytes = unsafe { CStr::from_ptr(buffer.as_ptr()) }.to_bytes();
    Ok(PathBuf::from(std::ffi::OsString::from_vec(bytes.to_vec())))
}

fn managed_id_component<'a>(value: &'a str, prefix: &str) -> Result<&'a str, ClewError> {
    let component = value
        .strip_prefix(prefix)
        .ok_or_else(|| invalid("managed identifier has the wrong prefix"))?;
    if component.is_empty()
        || component.len() > 128
        || !component
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(invalid("managed identifier is not a safe path component"));
    }
    Ok(component)
}

fn repository_key(repo: &Path) -> Result<String, ClewError> {
    let common = git(repo, &["rev-parse", "--git-common-dir"])
        .ok()
        .map(|value| {
            let path = PathBuf::from(value);
            if path.is_absolute() {
                path
            } else {
                repo.join(path)
            }
        })
        .and_then(|path| path.canonicalize().ok())
        .unwrap_or_else(|| repo.to_path_buf());
    let metadata = fs::metadata(&common).map_err(io_error)?;
    let object_format =
        git(repo, &["rev-parse", "--show-object-format"]).unwrap_or_else(|_| "sha1".to_owned());
    let root_commit = git(repo, &["rev-list", "--max-parents=0", "HEAD"])
        .ok()
        .and_then(|value| value.lines().next().map(str::to_owned))
        .unwrap_or_else(|| "NO_GIT_ROOT".to_owned());
    let mut digest = Sha256::new();
    digest.update(b"codeclew-repo/v1\0");
    #[cfg(unix)]
    {
        digest.update(metadata.dev().to_be_bytes());
        digest.update(metadata.ino().to_be_bytes());
    }
    #[cfg(not(unix))]
    digest.update(common.as_os_str().as_encoded_bytes());
    digest.update([0]);
    digest.update(object_format.as_bytes());
    digest.update([0]);
    digest.update(root_commit.as_bytes());
    Ok(hex::encode(digest.finalize()))
}

fn git(repo: &Path, arguments: &[&str]) -> Result<String, ClewError> {
    let mut command = Command::new("git");
    command
        .args([
            "--no-replace-objects",
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.attributesFile=/dev/null",
            "-c",
            "protocol.allow=never",
            "-c",
            "protocol.ext.allow=never",
            "-c",
            "protocol.file.allow=never",
        ])
        .args(arguments)
        .current_dir(repo)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_ATTR_NOSYSTEM", "1")
        .env("GIT_CONFIG_COUNT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("GIT_ALLOW_PROTOCOL", "");
    for name in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_COMMON_DIR",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_NAMESPACE",
        "GIT_REPLACE_REF_BASE",
    ] {
        command.env_remove(name);
    }
    let output = command.output().map_err(io_error)?;
    if !output.status.success() {
        return Err(invalid("repository identity is unavailable"));
    }
    String::from_utf8(output.stdout)
        .map(|value| value.trim().to_owned())
        .map_err(|_| invalid("Git repository identity is not UTF-8"))
}

pub(crate) fn create_private_directory(path: &Path) -> Result<(), ClewError> {
    if path.exists() {
        let metadata = fs::symlink_metadata(path).map_err(io_error)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(invalid("managed state path is not a real directory"));
        }
    } else {
        fs::create_dir_all(path).map_err(io_error)?;
    }
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(io_error)?;
    Ok(())
}

fn invalid(message: &str) -> ClewError {
    ClewError::new(ErrorCode::InvalidInput, message)
}

fn io_error(error: std::io::Error) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    use std::os::unix::fs::symlink;

    #[test]
    fn repository_state_is_external_private_and_path_free() {
        let home = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(home.path().join("state")).unwrap();
        let repository = state.repository(repo.path()).unwrap();
        assert!(repository.root.starts_with(state.root()));
        assert!(!repository.root.starts_with(repo.path()));
        assert_eq!(repository.key.len(), 64);
        assert!(
            !repository
                .key
                .contains(repo.path().to_string_lossy().as_ref())
        );
    }

    #[test]
    fn atomic_create_installs_complete_bytes_once_without_replacement() {
        let parent = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(parent.path().join("state")).unwrap();
        let directory = state.directory(Path::new("locks")).unwrap();
        let name = std::ffi::OsStr::new("immutable-root.json");
        assert!(
            directory
                .atomic_create(name, b"first-complete-value")
                .unwrap()
        );
        assert!(!directory.atomic_create(name, b"second-value").unwrap());
        assert_eq!(
            directory.read_file(name, 1024).unwrap(),
            b"first-complete-value"
        );
        assert!(
            directory
                .entries()
                .unwrap()
                .iter()
                .all(|entry| !entry.to_string_lossy().starts_with(".tmp-"))
        );
    }

    #[cfg(unix)]
    #[test]
    fn descendant_symlink_cannot_redirect_session_creation() {
        let parent = tempfile::tempdir().unwrap();
        let state_root = parent.path().join("state");
        let state = StateAuthority::open(state_root.clone()).unwrap();
        let victim = parent.path().join("victim");
        fs::create_dir(&victim).unwrap();
        fs::remove_dir(state_root.join("sessions")).unwrap();
        symlink(&victim, state_root.join("sessions")).unwrap();

        assert!(state.session_root("session:redirected").is_err());
        assert!(!victim.join("redirected").exists());
    }

    #[cfg(unix)]
    #[test]
    fn state_root_open_refuses_symlink_leaf() {
        let parent = tempfile::tempdir().unwrap();
        let victim = parent.path().join("victim");
        fs::create_dir(&victim).unwrap();
        symlink(&victim, parent.path().join("state")).unwrap();

        assert!(StateAuthority::open(parent.path().join("state")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn atomic_write_remains_bound_to_open_root_after_path_replacement() {
        let parent = tempfile::tempdir().unwrap();
        let state_root = parent.path().join("state");
        let moved_root = parent.path().join("state-open-inode");
        let state = StateAuthority::open(state_root.clone()).unwrap();
        let session = state.session_root("session:bound").unwrap();
        let receipt = session.join("receipt.json");

        fs::rename(&state_root, &moved_root).unwrap();
        fs::create_dir(&state_root).unwrap();
        fs::set_permissions(&state_root, fs::Permissions::from_mode(0o700)).unwrap();
        fs::create_dir(state_root.join("sessions")).unwrap();
        fs::set_permissions(
            state_root.join("sessions"),
            fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        fs::create_dir(state_root.join("sessions/bound")).unwrap();
        fs::set_permissions(
            state_root.join("sessions/bound"),
            fs::Permissions::from_mode(0o700),
        )
        .unwrap();

        state.write_private_atomic(&receipt, b"trusted").unwrap();
        assert_eq!(
            fs::read(moved_root.join("sessions/bound/receipt.json")).unwrap(),
            b"trusted"
        );
        assert!(!state_root.join("sessions/bound/receipt.json").exists());
    }

    #[cfg(unix)]
    #[test]
    fn atomic_write_refuses_symlink_target_without_touching_victim() {
        let parent = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(parent.path().join("state")).unwrap();
        let session = state.session_root("session:symlink-target").unwrap();
        let victim = parent.path().join("victim.json");
        fs::write(&victim, b"private").unwrap();
        let receipt = session.join("receipt.json");
        symlink(&victim, &receipt).unwrap();

        assert!(state.write_private_atomic(&receipt, b"forged").is_err());
        assert_eq!(fs::read(victim).unwrap(), b"private");
    }

    #[cfg(unix)]
    #[test]
    fn create_private_file_refuses_symlink_ancestor() {
        let parent = tempfile::tempdir().unwrap();
        let state_root = parent.path().join("state");
        let state = StateAuthority::open(state_root.clone()).unwrap();
        let victim = parent.path().join("victim");
        fs::create_dir(&victim).unwrap();
        fs::remove_dir(state_root.join("tmp")).unwrap();
        symlink(&victim, state_root.join("tmp")).unwrap();

        assert!(
            state
                .create_private_file(Path::new("tmp/escaped.json"))
                .is_err()
        );
        assert!(!victim.join("escaped.json").exists());
    }

    #[cfg(unix)]
    #[test]
    fn managed_attempt_and_compiler_stay_on_open_inode_after_root_replacement() {
        let parent = tempfile::tempdir().unwrap();
        let state_root = parent.path().join("state");
        let moved_root = parent.path().join("state-open-inode");
        let state = StateAuthority::open(state_root.clone()).unwrap();
        let attempts = state.directory(Path::new("attempts")).unwrap();
        let compiler = state
            .directory(Path::new("generations/compiler-store"))
            .unwrap()
            .child(Path::new("trusted"))
            .unwrap();

        fs::rename(&state_root, &moved_root).unwrap();
        fs::create_dir(&state_root).unwrap();
        fs::set_permissions(&state_root, fs::Permissions::from_mode(0o700)).unwrap();
        fs::create_dir(state_root.join("attempts")).unwrap();
        fs::create_dir_all(state_root.join("generations/compiler-store/trusted")).unwrap();

        let attempt = attempts.temporary_child("pinned-attempt").unwrap();
        let attempt_path = attempt.directory().resolved_path().unwrap();
        fs::write(attempt_path.join("marker"), b"attempt").unwrap();
        let compiler_path = compiler.resolved_path().unwrap();
        let status = Command::new("/bin/sh")
            .args(["-c", "printf compiler > \"$MANAGED/probe\""])
            .env("MANAGED", compiler_path)
            .status()
            .unwrap();
        assert!(status.success());

        assert!(
            attempt_path
                .canonicalize()
                .unwrap()
                .starts_with(moved_root.canonicalize().unwrap())
        );
        assert_eq!(
            fs::read(moved_root.join("generations/compiler-store/trusted/probe")).unwrap(),
            b"compiler"
        );
        assert!(
            !state_root
                .join("generations/compiler-store/trusted/probe")
                .exists()
        );
    }

    #[cfg(unix)]
    #[test]
    fn managed_attempt_and_compiler_children_refuse_symlink_substitution() {
        let parent = tempfile::tempdir().unwrap();
        let state_root = parent.path().join("state");
        let state = StateAuthority::open(state_root.clone()).unwrap();
        let victim = parent.path().join("victim");
        fs::create_dir(&victim).unwrap();

        fs::remove_dir(state_root.join("attempts")).unwrap();
        symlink(&victim, state_root.join("attempts")).unwrap();
        assert!(state.directory(Path::new("attempts/forged")).is_err());
        assert!(!victim.join("forged").exists());

        fs::remove_file(state_root.join("attempts")).unwrap();
        fs::create_dir(state_root.join("attempts")).unwrap();
        symlink(&victim, state_root.join("generations/compiler-store")).unwrap();
        assert!(
            state
                .directory(Path::new("generations/compiler-store/forged"))
                .is_err()
        );
        assert!(!victim.join("forged").exists());
    }
}
