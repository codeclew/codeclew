//! Small POSIX primitives for anchored sidecar maintenance.
//!
//! All names accepted here are one directory component.  Callers keep the
//! returned directory descriptor alive while they inspect and mutate entries;
//! no path is re-resolved from the process working directory.

use super::super::{invalid, io_error};
use crate::error::ClewError;
use std::ffi::CString;
use std::fs::{File, Metadata, OpenOptions};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;

#[derive(Debug)]
pub(super) struct Dir(File);

fn component(name: &str) -> Result<CString, ClewError> {
    if name.is_empty() || name == "." || name == ".." || name.contains('/') || name.contains('\0') {
        return Err(invalid("anchored sidecar path component is unsafe"));
    }
    CString::new(name).map_err(|_| invalid("anchored sidecar path component is unsafe"))
}

fn current_uid() -> u32 {
    unsafe { libc::geteuid() }
}

fn owned(metadata: &Metadata) -> bool {
    metadata.uid() == current_uid()
}

fn open_at(parent: RawFd, name: &CString, flags: i32, mode: u32) -> Result<File, ClewError> {
    let fd = unsafe { libc::openat(parent, name.as_ptr(), flags, mode) };
    if fd < 0 {
        return Err(io_error(io::Error::last_os_error()));
    }
    // SAFETY: openat returned a fresh owned descriptor.
    Ok(unsafe { File::from_raw_fd(fd) })
}

impl Dir {
    pub(super) fn open_root(path: &Path) -> Result<Self, ClewError> {
        let mut options = OpenOptions::new();
        options
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC);
        let file = options.open(path).map_err(io_error)?;
        let metadata = file.metadata().map_err(io_error)?;
        if !metadata.is_dir() || !owned(&metadata) {
            return Err(invalid("anchored sidecar root is not an owned directory"));
        }
        Ok(Self(file))
    }

    pub(super) fn open_dir(&self, name: &str) -> Result<Self, ClewError> {
        let name = component(name)?;
        let file = open_at(
            self.0.as_raw_fd(),
            &name,
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0,
        )?;
        let metadata = file.metadata().map_err(io_error)?;
        if !metadata.is_dir() || !owned(&metadata) {
            return Err(invalid("anchored sidecar child is not an owned directory"));
        }
        Ok(Self(file))
    }

    pub(super) fn private_dir(&self, name: &str) -> Result<Self, ClewError> {
        let name = component(name)?;
        let result = unsafe { libc::mkdirat(self.0.as_raw_fd(), name.as_ptr(), 0o700) };
        if result != 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::EEXIST) {
                return Err(io_error(error));
            }
        }
        let file = open_at(
            self.0.as_raw_fd(),
            &name,
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0,
        )?;
        let metadata = file.metadata().map_err(io_error)?;
        if !metadata.is_dir() || !owned(&metadata) || metadata.mode() & 0o7777 != 0o700 {
            return Err(invalid(
                "anchored sidecar private directory is not owned mode 0700",
            ));
        }
        Ok(Self(file))
    }

    pub(super) fn open_file(&self, name: &str, limit: u64) -> Result<File, ClewError> {
        let name = component(name)?;
        let file = open_at(
            self.0.as_raw_fd(),
            &name,
            libc::O_RDONLY | libc::O_NONBLOCK | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0,
        )?;
        let metadata = file.metadata().map_err(io_error)?;
        if !metadata.is_file() || !owned(&metadata) || metadata.len() > limit {
            return Err(invalid(
                "anchored sidecar file is not an owned bounded regular file",
            ));
        }
        Ok(file)
    }

    pub(super) fn lock_file(&self, name: &str) -> Result<File, ClewError> {
        let name = component(name)?;
        let file = open_at(
            self.0.as_raw_fd(),
            &name,
            libc::O_RDWR | libc::O_CREAT | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
            0o600,
        )?;
        let metadata = file.metadata().map_err(io_error)?;
        if !metadata.is_file() || !owned(&metadata) || metadata.mode() & 0o7777 != 0o600 {
            return Err(invalid(
                "anchored sidecar lock is not an owned mode 0600 file",
            ));
        }
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(invalid("another sidecar compaction operation is active"));
        }
        Ok(file)
    }

    pub(super) fn capture(
        &self,
        name: &str,
        stage: &Dir,
        stage_name: &str,
    ) -> Result<(), ClewError> {
        let name = component(name)?;
        let stage_name = component(stage_name)?;
        if stage.exists_nofollow(stage_name.to_str().unwrap_or_default())? {
            return Err(invalid("anchored sidecar staging slot is occupied"));
        }
        #[cfg(target_os = "linux")]
        {
            let result = unsafe {
                renameat2(
                    self.0.as_raw_fd(),
                    name.as_ptr(),
                    stage.0.as_raw_fd(),
                    stage_name.as_ptr(),
                    RENAME_NOREPLACE,
                )
            };
            if result == 0 {
                return Ok(());
            }
            Err(io_error(io::Error::last_os_error()))
        }
        #[cfg(target_os = "macos")]
        {
            let result = unsafe {
                renameatx_np(
                    self.0.as_raw_fd(),
                    name.as_ptr(),
                    stage.0.as_raw_fd(),
                    stage_name.as_ptr(),
                    RENAME_EXCL,
                )
            };
            if result == 0 {
                return Ok(());
            }
            Err(io_error(io::Error::last_os_error()))
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            let _ = (name, stage_name);
            Err(invalid(
                "anchored sidecar capture is unsupported on this platform",
            ))
        }
    }

    pub(super) fn unlink(&self, name: &str) -> Result<(), ClewError> {
        let name = component(name)?;
        if unsafe { libc::unlinkat(self.0.as_raw_fd(), name.as_ptr(), 0) } != 0 {
            return Err(io_error(io::Error::last_os_error()));
        }
        Ok(())
    }

    pub(super) fn metadata(&self) -> Result<Metadata, ClewError> {
        self.0.metadata().map_err(io_error)
    }

    pub(super) fn exists_nofollow(&self, name: &str) -> Result<bool, ClewError> {
        let name = component(name)?;
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        let result = unsafe {
            libc::fstatat(
                self.0.as_raw_fd(),
                name.as_ptr(),
                stat.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        if result == 0 {
            return Ok(true);
        }
        if io::Error::last_os_error().raw_os_error() == Some(libc::ENOENT) {
            Ok(false)
        } else {
            Err(io_error(io::Error::last_os_error()))
        }
    }
}

#[cfg(target_os = "linux")]
const RENAME_NOREPLACE: u32 = 1;

#[cfg(target_os = "linux")]
unsafe extern "C" {
    fn renameat2(
        olddirfd: libc::c_int,
        oldpath: *const libc::c_char,
        newdirfd: libc::c_int,
        newpath: *const libc::c_char,
        flags: libc::c_uint,
    ) -> libc::c_int;
}

#[cfg(target_os = "macos")]
const RENAME_EXCL: u32 = 0x00000004;

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn renameatx_np(
        olddirfd: libc::c_int,
        oldpath: *const libc::c_char,
        newdirfd: libc::c_int,
        newpath: *const libc::c_char,
        flags: libc::c_uint,
    ) -> libc::c_int;
}
