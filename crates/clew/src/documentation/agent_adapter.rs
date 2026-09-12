//! A narrow stdio adapter backed by the existing macOS Seatbelt facility.
//! A normal child process is never an admitted fallback.
use super::{agent_jobs::Role, analysis, bytes, digest, invalid, io_error, store::Repository};
use crate::error::ClewError;
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

pub struct Execution {
    pub output: Option<Value>,
    pub failure: Option<String>,
    pub stdout_bytes: usize,
    pub stderr_bytes: usize,
    pub admission: Value,
}
fn quoted(path: &Path) -> Result<String, ClewError> {
    serde_json::to_string(
        path.to_str()
            .ok_or_else(|| invalid("adapter path is not UTF-8"))?,
    )
    .map_err(io_error)
}
fn overlaps(a: &Path, b: &Path) -> bool {
    a.starts_with(b) || b.starts_with(a)
}
pub fn admit(repo: &Repository, role: &Role) -> Result<Value, ClewError> {
    if !cfg!(target_os = "macos")
        || role.adapter != "macos-seatbelt-stdio/1.0"
        || !Path::new("/usr/bin/sandbox-exec").is_file()
    {
        return Err(invalid(
            "ISOLATION_UNAVAILABLE: configure a supported isolated stdio adapter for this host",
        ));
    }
    if !matches!(
        role.usage_authority.as_str(),
        "MAXIMUM_ONLY" | "TRANSPORT_METADATA"
    ) || role.model.trim().is_empty()
        || role.model.len() > 256
        || role.command.is_empty()
        || role.command.len() > 32
        || role.runtime_reads.len() > 64
        || !role.cap.maximum.positive()
        || role.cap.timeout_ms == 0
        || role.cap.timeout_ms > 600_000
        || role.cap.output_bytes == 0
        || role.cap.output_bytes > 2 * 1024 * 1024
    {
        return Err(invalid("invalid role command or finite per-call caps"));
    }
    let program = Path::new(&role.command[0]);
    if !program.is_absolute() || !program.is_file() {
        return Err(invalid(
            "adapter driver requires an absolute executable path",
        ));
    }
    let mut protected = vec![repo.root.clone()];
    for service in repo.services()?.values() {
        if let Ok(path) = analysis::bound_repository(repo, service) {
            protected.push(path);
        }
    }
    for runtime in [
        "/System",
        "/usr/lib",
        "/usr/share",
        "/Library/Apple",
        "/private/var/db/dyld",
    ] {
        if protected.iter().any(|p| overlaps(Path::new(runtime), p)) {
            return Err(invalid(
                "ISOLATION_SCOPE_CONFLICT: protected input overlaps the OS runtime allowance",
            ));
        }
    }
    let mut registered = role.runtime_reads.clone();
    registered.push(program.to_path_buf());
    let mut inventory = Vec::new();
    for path in registered {
        if !path.is_absolute() {
            return Err(invalid("runtime inputs require absolute paths"));
        }
        let canonical = path.canonicalize().map_err(io_error)?;
        if protected.iter().any(|p| overlaps(&canonical, p)) || canonical.parent().is_none() {
            return Err(invalid(
                "ISOLATION_SCOPE_CONFLICT: runtime reads overlap source, documentation or coordinator state",
            ));
        }
        let meta = fs::metadata(&canonical).map_err(io_error)?;
        if meta.is_file() && meta.len() > 64 * 1024 * 1024 {
            return Err(invalid("registered driver file exceeds 64 MiB"));
        }
        if !meta.is_dir() && !meta.is_file() {
            return Err(invalid("runtime input is not a regular file or directory"));
        }
        inventory.push(json!({"path":canonical,"kind":if meta.is_dir(){"REGISTERED_RUNTIME_DIRECTORY"}else{"REGISTERED_DRIVER_FILE"},"digest":if meta.is_file(){Some(crate::canonical::hash_bytes(&fs::read(&canonical).map_err(io_error)?))}else{None}}));
    }
    if role.environment.len() > 16
        || role.environment.iter().any(|name| {
            name.is_empty()
                || name.len() > 100
                || !name
                    .bytes()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'_')
                || ["HOME", "PATH", "TMPDIR"].contains(&name.as_str())
                || ["CODECLEW_", "DYLD_", "LD_", "PYTHON"]
                    .iter()
                    .any(|prefix| name.starts_with(prefix))
        })
    {
        return Err(invalid(
            "driver environment permits only explicit credential/configuration names",
        ));
    }
    for name in &role.environment {
        if std::env::var_os(name).is_none() {
            return Err(invalid(
                "CONFIGURATION_MISSING: a configured driver environment value is absent",
            ));
        }
    }
    Ok(
        json!({"schema":"codeclew-documentation-isolation/1.0","adapter":role.adapter,"driverDigest":digest(&(role.command.clone(),&inventory,&role.environment,role.network,&role.model,&role.usage_authority))?,"capabilities":{"workInput":"IMMUTABLE_STDIN","expansion":"COORDINATOR_REGISTERED_ONLY","result":"ROLE_STDOUT_ONLY","sourceWrites":false,"humanWrites":false,"coordinatorWrites":false,"crossRoleWrites":false,"unregisteredFileReads":false,"subprocessTools":false,"network":role.network}}),
    )
}
fn profile(role: &Role, cwd: &Path) -> Result<String, ClewError> {
    let mut clauses = Vec::new();
    for literal in [
        "/",
        "/private",
        "/private/var",
        "/dev/null",
        "/dev/urandom",
        "/dev/random",
        "/private/etc/localtime",
    ] {
        clauses.push(format!("(literal {})", quoted(Path::new(literal))?));
    }
    clauses.push(format!("(literal {})", quoted(cwd)?));
    for directory in [
        "/System",
        "/usr/lib",
        "/usr/share",
        "/Library/Apple",
        "/private/var/db/dyld",
    ] {
        clauses.push(format!("(subpath {})", quoted(Path::new(directory))?));
    }
    for path in role
        .runtime_reads
        .iter()
        .cloned()
        .chain(std::iter::once(PathBuf::from(&role.command[0])))
    {
        let canonical = path.canonicalize().map_err(io_error)?;
        for path in [path, canonical] {
            clauses.push(format!(
                "({} {})",
                if path.is_dir() { "subpath" } else { "literal" },
                quoted(&path)?
            ));
        }
    }
    let allowed = format!("(require-any {})", clauses.join(" "));
    let executable = Path::new(&role.command[0]);
    let canonical = executable.canonicalize().map_err(io_error)?;
    Ok(format!(
        "(version 1) (allow default) (deny file-read-data (require-not {allowed})) (deny file-map-executable (require-not {allowed})) (deny file-write*) (deny process-fork) (deny process-exec (require-not (require-any (literal {}) (literal {})))) {}",
        quoted(executable)?,
        quoted(&canonical)?,
        if role.network { "" } else { "(deny network*)" }
    ))
}
#[cfg(unix)]
fn nonblocking(fd: std::os::fd::RawFd) -> Result<(), ClewError> {
    // SAFETY: fcntl only changes flags on the owned pipe descriptor.
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        if flags < 0 || libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) < 0 {
            return Err(io_error("pipe flags"));
        }
    }
    Ok(())
}
fn drain(reader: &mut impl Read, buffer: &mut Vec<u8>, cap: usize) -> Result<(), ClewError> {
    let mut chunk = [0u8; 8192];
    loop {
        match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                if buffer.len().saturating_add(n) > cap {
                    return Err(invalid("OUTPUT_CAP_EXCEEDED"));
                }
                buffer.extend_from_slice(&chunk[..n]);
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return Err(io_error("driver pipe")),
        }
    }
    Ok(())
}
struct RunningChild(std::process::Child);
impl std::ops::Deref for RunningChild {
    type Target = std::process::Child;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for RunningChild {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
impl Drop for RunningChild {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}
#[cfg(unix)]
pub fn execute(
    repo: &Repository,
    role: &Role,
    input: &Value,
    cancel: &Path,
) -> Result<Execution, ClewError> {
    use std::os::{fd::AsRawFd, unix::process::CommandExt};
    let admission = admit(repo, role)?;
    let data = bytes(input)?;
    if (data.len() as u64)
        .checked_add(role.cap.overhead_input_tokens)
        .is_none_or(|n| n > role.cap.maximum.input_tokens)
    {
        return Err(invalid(
            "INPUT_CAP_EXCEEDED: request bytes plus configured transport overhead exceed the conservative token upper bound",
        ));
    }
    let temp = tempfile::tempdir().map_err(io_error)?;
    let cwd = temp.path().canonicalize().map_err(io_error)?;
    let mut command = Command::new("/usr/bin/sandbox-exec");
    command
        .args(["-p", &profile(role, &cwd)?])
        .args(&role.command)
        .current_dir(&cwd)
        .env_clear()
        .env("HOME", &cwd)
        .env("TMPDIR", &cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    for name in &role.environment {
        command.env(
            name,
            std::env::var_os(name).ok_or_else(|| invalid("driver environment disappeared"))?,
        );
    }
    // SAFETY: the child callback performs only async-signal-safe descriptor syscalls.
    // Mark every inherited descriptor close-on-exec, including launcher authority FDs.
    unsafe {
        command.pre_exec(|| {
            let limit = libc::sysconf(libc::_SC_OPEN_MAX);
            for fd in 3..limit.max(3) {
                libc::fcntl(fd as i32, libc::F_SETFD, libc::FD_CLOEXEC);
            }
            Ok(())
        });
    }
    let mut child = RunningChild(
        command
            .spawn()
            .map_err(|_| invalid("ISOLATED_DRIVER_START_FAILED"))?,
    );
    let mut stdin = Some(child.stdin.take().ok_or_else(|| io_error("stdin"))?);
    let mut stdout = child.stdout.take().ok_or_else(|| io_error("stdout"))?;
    let mut stderr = child.stderr.take().ok_or_else(|| io_error("stderr"))?;
    nonblocking(stdin.as_ref().unwrap().as_raw_fd())?;
    nonblocking(stdout.as_raw_fd())?;
    nonblocking(stderr.as_raw_fd())?;
    let mut sent = 0;
    let mut output = Vec::new();
    let mut errors = Vec::new();
    let start = Instant::now();
    let mut failure = None;
    loop {
        if cancel.exists() {
            failure = Some("CANCELLED".into());
            break;
        }
        if start.elapsed() >= Duration::from_millis(role.cap.timeout_ms) {
            failure = Some("TIME_CAP_EXCEEDED".into());
            break;
        }
        if let Some(writer) = stdin.as_mut() {
            match writer.write(&data[sent..]) {
                Ok(n) => sent += n,
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                    ) => {}
                Err(_) => {
                    stdin = None;
                }
            }
        }
        if sent == data.len() {
            stdin = None;
        }
        if let Err(error) = drain(&mut stdout, &mut output, role.cap.output_bytes)
            .and_then(|_| drain(&mut stderr, &mut errors, role.cap.output_bytes))
        {
            failure = Some(error.message);
            break;
        }
        if let Some(status) = child.try_wait().map_err(io_error)? {
            if !status.success() {
                failure = Some("ISOLATED_DRIVER_FAILED".into());
            }
            // The process cannot fork, so EOF follows after its remaining pipe bytes.
            if let Err(error) = drain(&mut stdout, &mut output, role.cap.output_bytes) {
                failure = Some(error.message);
            }
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    if failure.is_some() {
        let _ = child.kill();
        let _ = child.wait();
    }
    let parsed = if failure.is_none() {
        match serde_json::from_slice(&output) {
            Ok(value) => Some(value),
            Err(_) => {
                failure = Some("MALFORMED_DRIVER_OUTPUT".into());
                None
            }
        }
    } else {
        None
    };
    Ok(Execution {
        output: parsed,
        failure,
        stdout_bytes: output.len(),
        stderr_bytes: errors.len(),
        admission,
    })
}
#[cfg(not(unix))]
pub fn execute(_: &Repository, _: &Role, _: &Value, _: &Path) -> Result<Execution, ClewError> {
    Err(invalid("ISOLATION_UNAVAILABLE"))
}
