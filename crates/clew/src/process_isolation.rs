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
    use std::process::Stdio;
    use std::sync::Mutex;

    static FD_TEST_LOCK: Mutex<()> = Mutex::new(());

    fn duplicate_for_child(descriptor: RawFd) -> RawFd {
        let duplicate = unsafe { libc::fcntl(descriptor, libc::F_DUPFD, 100) };
        assert!(duplicate >= 100);
        duplicate
    }

    #[test]
    fn child_receives_neither_controller_env_nor_authority_fd() {
        let _guard = FD_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut pipe = [0 as RawFd; 2];
        assert_eq!(unsafe { libc::pipe(pipe.as_mut_ptr()) }, 0);
        let authority_fd = duplicate_for_child(pipe[1]);
        assert_eq!(unsafe { libc::close(pipe[1]) }, 0);
        let mut command = Command::new("/bin/sh");
        command
            .args([
                "-c",
                "test -z \"${CODECLEW_STATE_ROOT_FD+x}\" && ! eval 'printf x >&'$TEST_FD",
            ])
            .stderr(Stdio::null())
            .env("CODECLEW_STATE_ROOT_FD", authority_fd.to_string())
            .env("TEST_FD", authority_fd.to_string());
        isolate_with_environment(
            &mut command,
            [(
                OsString::from("CODECLEW_STATE_ROOT_FD"),
                authority_fd.to_string().into(),
            )],
        )
        .unwrap();
        assert!(command.status().unwrap().success());
        unsafe {
            libc::close(pipe[0]);
            libc::close(authority_fd);
        }
    }

    #[test]
    fn child_closes_every_controller_fd_but_preserves_unrelated_descriptors() {
        let _guard = FD_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut state_pipe = [0 as RawFd; 2];
        let mut runtime_pipe = [0 as RawFd; 2];
        let mut lease_pipe = [0 as RawFd; 2];
        let mut unrelated_pipe = [0 as RawFd; 2];
        for pipe in [
            &mut state_pipe,
            &mut runtime_pipe,
            &mut lease_pipe,
            &mut unrelated_pipe,
        ] {
            assert_eq!(unsafe { libc::pipe(pipe.as_mut_ptr()) }, 0);
        }
        let state_fd = duplicate_for_child(state_pipe[1]);
        let runtime_fd = duplicate_for_child(runtime_pipe[1]);
        let lease_fd = duplicate_for_child(lease_pipe[1]);
        let unrelated_fd = duplicate_for_child(unrelated_pipe[1]);
        for pipe in [state_pipe, runtime_pipe, lease_pipe, unrelated_pipe] {
            assert_eq!(unsafe { libc::close(pipe[1]) }, 0);
        }
        let authority = [
            ("CODECLEW_STATE_ROOT_FD", state_fd),
            ("CODECLEW_RUNTIME_ROOT_FD", runtime_fd),
            ("CODECLEW_RUNTIME_LEASE_FD", lease_fd),
        ];
        let mut command = Command::new("/bin/sh");
        command
            .args([
                "-c",
                concat!(
                    "test -z \"${CODECLEW_STATE_ROOT_FD+x}\" && ",
                    "test -z \"${CODECLEW_RUNTIME_ROOT_FD+x}\" && ",
                    "test -z \"${CODECLEW_RUNTIME_LEASE_FD+x}\" && ",
                    "! eval 'printf x >&'$STATE_FD && ",
                    "! eval 'printf x >&'$RUNTIME_FD && ",
                    "! eval 'printf x >&'$LEASE_FD && ",
                    "eval 'printf ok >&'$UNRELATED_FD"
                ),
            ])
            .stderr(Stdio::null())
            .env("STATE_FD", state_fd.to_string())
            .env("RUNTIME_FD", runtime_fd.to_string())
            .env("LEASE_FD", lease_fd.to_string())
            .env("UNRELATED_FD", unrelated_fd.to_string());
        for (name, descriptor) in &authority {
            command.env(name, descriptor.to_string());
        }
        isolate_with_environment(
            &mut command,
            authority
                .map(|(name, descriptor)| (OsString::from(name), descriptor.to_string().into())),
        )
        .unwrap();
        assert!(command.status().unwrap().success());
        let mut bytes = [0_u8; 2];
        assert_eq!(
            unsafe { libc::read(unrelated_pipe[0], bytes.as_mut_ptr().cast(), bytes.len(),) },
            2
        );
        assert_eq!(&bytes, b"ok");
        for descriptor in [
            state_pipe[0],
            runtime_pipe[0],
            lease_pipe[0],
            unrelated_pipe[0],
            state_fd,
            runtime_fd,
            lease_fd,
            unrelated_fd,
        ] {
            unsafe {
                libc::close(descriptor);
            }
        }
    }
}
