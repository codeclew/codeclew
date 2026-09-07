//! Maven launch configuration shared by Java readiness and model extraction.
//! External settings stay private and are never stored in the source CAS.
use crate::canonical;
use crate::error::{ClewError, ErrorCode};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

const MAX_SETTINGS_BYTES: u64 = 1024 * 1024;

pub fn command(repository: &Path) -> Result<Command, ClewError> {
    let wrapper = repository.join("mvnw");
    if wrapper.exists() {
        if !fs::symlink_metadata(&wrapper).is_ok_and(|metadata| metadata.is_file()) {
            return Err(launcher_error());
        }
        if executable(&wrapper) {
            return Ok(Command::new(wrapper));
        }
        // The source snapshot retains its original mode. A known shell wrapper
        // does not need chmod, a preparatory commit, or a different Maven version.
        let mut prefix = Vec::new();
        File::open(&wrapper)
            .and_then(|file| file.take(256).read_to_end(&mut prefix))
            .map_err(|_| launcher_error())?;
        let line = prefix.split(|byte| *byte == b'\n').next().unwrap_or(&[]);
        let interpreter = match line.strip_suffix(b"\r").unwrap_or(line) {
            b"#!/bin/sh" | b"#!/usr/bin/env sh" => "/bin/sh",
            b"#!/bin/bash" | b"#!/usr/bin/env bash" => "/bin/bash",
            _ => return Err(launcher_error()),
        };
        if !executable(Path::new(interpreter)) {
            return Err(launcher_error());
        }
        let mut command = Command::new(interpreter);
        command.arg(wrapper);
        return Ok(command);
    }
    let launcher = std::env::var_os("PATH")
        .and_then(|paths| {
            std::env::split_paths(&paths)
                .map(|directory| directory.join("mvn"))
                .find(|path| executable(path))
        })
        .ok_or_else(launcher_error)?;
    Ok(Command::new(launcher))
}

fn executable(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|metadata| {
        metadata.is_file() && {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                metadata.permissions().mode() & 0o111 != 0
            }
            #[cfg(not(unix))]
            {
                true
            }
        }
    })
}

fn launcher_error() -> ClewError {
    ClewError::new(
        ErrorCode::UnsupportedProjectConfiguration,
        "BUILD_LAUNCHER_START_FAILED: Maven requires the project wrapper or Maven on PATH. Non-executable mvnw scripts with a supported sh/bash shebang run through their interpreter without chmod or a commit. Restore an unreadable or unsupported wrapper; Codeclew does not substitute another Maven when mvnw is present.",
    )
}

/// Private operational locator. Only its digest is exposed by the session.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MavenSettings {
    pub path: PathBuf,
    pub digest: String,
}

impl MavenSettings {
    pub fn capture(path: &Path) -> Result<Self, ClewError> {
        let path = path.canonicalize().map_err(|_| settings_error())?;
        let bytes = read_settings(&path)?;
        Ok(Self {
            path,
            digest: canonical::hash_bytes(&bytes),
        })
    }

    /// Both Maven calls use the same mode-0600 bytes, even if the original file
    /// changes during extraction. A later extraction must match the session.
    pub fn materialize(&self) -> Result<tempfile::NamedTempFile, ClewError> {
        let bytes = read_settings(&self.path)?;
        if canonical::hash_bytes(&bytes) != self.digest {
            return Err(ClewError::new(
                ErrorCode::InputMutated,
                "Maven settings changed since session admission; open a new session with --maven-settings.",
            ));
        }
        let mut file = tempfile::NamedTempFile::new().map_err(|_| settings_error())?;
        file.write_all(&bytes).map_err(|_| settings_error())?;
        Ok(file)
    }
}

fn read_settings(path: &Path) -> Result<Vec<u8>, ClewError> {
    let file = File::open(path).map_err(|_| settings_error())?;
    let metadata = file.metadata().map_err(|_| settings_error())?;
    if !metadata.is_file() || metadata.len() > MAX_SETTINGS_BYTES {
        return Err(settings_error());
    }
    let mut bytes = Vec::new();
    file.take(MAX_SETTINGS_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| settings_error())?;
    if bytes.is_empty() || bytes.len() as u64 > MAX_SETTINGS_BYTES {
        return Err(settings_error());
    }
    Ok(bytes)
}

fn settings_error() -> ClewError {
    ClewError::new(
        ErrorCode::InvalidInput,
        "Maven settings must be a readable non-empty regular file of at most 1 MiB; pass --maven-settings with the intended settings.xml. Its contents and path are private.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(unix)]
    fn non_executable_wrapper_runs_without_changing_source_or_mode() {
        use std::os::unix::fs::PermissionsExt;
        let repo = tempfile::tempdir().unwrap();
        let wrapper = repo.path().join("mvnw");
        let source = b"#!/bin/sh\nprintf '%s' \"$1\"\n";
        fs::write(&wrapper, source).unwrap();
        fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o644)).unwrap();
        let output = command(repo.path())
            .unwrap()
            .arg("project-wrapper")
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"project-wrapper");
        assert_eq!(fs::read(&wrapper).unwrap(), source);
        assert_eq!(
            fs::metadata(&wrapper).unwrap().permissions().mode() & 0o777,
            0o644
        );
        fs::write(&wrapper, b"#!/unknown/interpreter\n").unwrap();
        assert!(command(repo.path()).is_err());
    }

    #[test]
    fn settings_are_pinned_private_and_do_not_overwrite_the_original() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.xml");
        let bytes = b"<settings><servers>private-value</servers></settings>";
        fs::write(&path, bytes).unwrap();
        let binding = MavenSettings::capture(&path).unwrap();
        let copy = binding.materialize().unwrap();
        assert_eq!(fs::read(copy.path()).unwrap(), bytes);
        assert_eq!(fs::read(&path).unwrap(), bytes);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                copy.as_file().metadata().unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        fs::write(&path, b"<settings/>").unwrap();
        let error = binding.materialize().unwrap_err();
        assert_eq!(error.code, ErrorCode::InputMutated);
        assert!(!error.message.contains("private-value"));
        assert!(!error.message.contains(dir.path().to_str().unwrap()));
        assert_eq!(fs::read(copy.path()).unwrap(), bytes);
    }
}
