use crate::error::{ClewError, ErrorCode};
use std::ffi::{OsStr, OsString};
use std::process::Command;

const AUTHORITY_FD_ENV: [&str; 3] = [
    "CODECLEW_STATE_ROOT_FD",
    "CODECLEW_RUNTIME_ROOT_FD",
    "CODECLEW_RUNTIME_LEASE_FD",
];

/// Remove controller capabilities before executing project or worker code.
///
/// The launcher intentionally gives the Rust supervisor three inherited file
/// descriptors. They are capabilities, not ambient build configuration. Every
/// child outside the supervisor contour must receive neither their names nor
/// live copies of the descriptors.
pub fn isolate_controller_authority(command: &mut Command) -> Result<(), ClewError> {
    isolate_with_environment(command, std::env::vars_os())
}

fn isolate_with_environment(
    command: &mut Command,
    environment: impl IntoIterator<Item = (OsString, OsString)>,
) -> Result<(), ClewError> {
    let mut descriptors = Vec::new();
    for (name, value) in environment {
        if name.to_string_lossy().starts_with("CODECLEW_") {
            command.env_remove(&name);
        }
        if AUTHORITY_FD_ENV
            .iter()
            .any(|candidate| name == OsStr::new(candidate))
        {
            let value = value
                .to_str()
                .ok_or_else(|| invalid("controller descriptor is not UTF-8"))?;
            let descriptor = value
                .parse::<i32>()
                .map_err(|_| invalid("controller descriptor is invalid"))?;
            if descriptor < 3 {
                return Err(invalid("controller descriptor is unsafe"));
            }
            descriptors.push(descriptor);
        }
    }
    descriptors.sort_unstable();
    descriptors.dedup();
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: close(2) is async-signal-safe and the closure captures only a
        // preallocated integer vector. EBADF is harmless if two authorities
        // referred to the same already-closed descriptor.
        unsafe {
            command.pre_exec(move || {
                for descriptor in &descriptors {
                    libc::close(*descriptor);
                }
                Ok(())
            });
        }
    }
    #[cfg(not(unix))]
    if !descriptors.is_empty() {
        return Err(invalid("controller descriptor isolation requires POSIX"));
    }
    Ok(())
}

fn invalid(message: &str) -> ClewError {
    ClewError::new(ErrorCode::InvalidInput, message)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::fd::RawFd;

    #[test]
    fn child_receives_neither_controller_env_nor_authority_fd() {
        let mut pipe = [0 as RawFd; 2];
        assert_eq!(unsafe { libc::pipe(pipe.as_mut_ptr()) }, 0);
        let mut command = Command::new("/bin/sh");
        command
            .args([
                "-c",
                "test -z \"${CODECLEW_STATE_ROOT_FD+x}\" && ! eval 'printf x >&'$TEST_FD",
            ])
            .env("CODECLEW_STATE_ROOT_FD", pipe[1].to_string())
            .env("TEST_FD", pipe[1].to_string());
        isolate_with_environment(
            &mut command,
            [(
                OsString::from("CODECLEW_STATE_ROOT_FD"),
                pipe[1].to_string().into(),
            )],
        )
        .unwrap();
        assert!(command.status().unwrap().success());
        unsafe {
            libc::close(pipe[0]);
            libc::close(pipe[1]);
        }
    }
}
