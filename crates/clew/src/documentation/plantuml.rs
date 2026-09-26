//! Shared PlantUML output helpers for auto-generated process diagrams.

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

const RENDERER_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_RENDERER_INPUT_BYTES: usize = 64 * 1024 * 1024;
const MAX_RENDERER_STDOUT_BYTES: usize = 32 * 1024 * 1024;
const MAX_RENDERER_STDERR_BYTES: usize = 1024 * 1024;
const MAX_RENDERED_SVG_BYTES: u64 = 32 * 1024 * 1024;
const MAX_RENDERED_TOTAL_BYTES: u64 = 128 * 1024 * 1024;
const MAX_BATCH_DIAGRAMS: usize = 4096;

/// Escape user-authored text for insertion into a single PlantUML label line.
///
/// PlantUML actions use `;` as a statement terminator, and state/activity
/// labels accept quoting, bracket, and parenthesis syntax. Replace those
/// delimiters with their visually similar Unicode forms and flatten every line
/// separator/control character so text cannot add PlantUML source lines.
pub fn escape(s: &str) -> String {
    let mut escaped = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push('＂'),
            '[' => escaped.push('［'),
            ']' => escaped.push('］'),
            '(' => escaped.push('（'),
            ')' => escaped.push('）'),
            ';' => escaped.push('；'),
            '\r' | '\n' | '\u{0085}' | '\u{2028}' | '\u{2029}' => escaped.push(' '),
            c if c.is_control() => escaped.push(' '),
            c => escaped.push(c),
        }
    }
    escaped
}

/// Emit the shared diagram header: plain theme + smetana layout + title.
pub fn header(title: &str) -> String {
    format!(
        "@startuml\n!theme plain\n!pragma layout smetana\ntitle {}\n",
        escape(title)
    )
}

/// Wrap a body in the PlantUML document envelope.
pub fn wrap(title: &str, body: &str) -> String {
    format!("{} {}\n@enduml\n", header(title), body.trim())
}

/// Attempt to pre-render PlantUML source to SVG bytes using an external
/// renderer. When `jar` is set it runs `java -jar <jar>`, otherwise it looks
/// for a `plantuml` binary on PATH. Returns `Ok(None)` when no renderer is
/// available, and `Err` on an execution failure, timeout, or output limit.
pub fn render_svg(puml: &[u8], jar: Option<&Path>) -> Result<Option<Vec<u8>>, String> {
    let Some(mut cmd) = renderer_command(jar) else {
        return Ok(None);
    };
    cmd.args(["-tsvg", "-charset", "UTF-8", "-pipe"]);
    secure_renderer(&mut cmd);
    let output = run_bounded(&mut cmd, Some(puml), RENDERER_TIMEOUT)?;
    ensure_success(&output.status, &output.stderr)?;
    Ok(Some(output.stdout))
}

type RenderedSvgArtifacts = Vec<(String, Vec<u8>)>;

/// Pre-render many PlantUML sources to SVG in a single renderer invocation.
/// Each entry is `(bundle-relative base path, puml source)`; the returned bytes
/// retain those same keys. Inputs use ordinal temporary filenames, so equal
/// basenames in different bundle directories cannot overwrite one another.
/// Returns `Ok(None)` when no renderer is available.
pub fn batch_render_svg(
    sources: &[(String, String)],
    jar: Option<&Path>,
) -> Result<Option<RenderedSvgArtifacts>, String> {
    let Some(cmd) = renderer_command(jar) else {
        return Ok(None);
    };
    batch_render_svg_with_command(sources, cmd).map(Some)
}

fn batch_render_svg_with_command(
    sources: &[(String, String)],
    mut cmd: Command,
) -> Result<RenderedSvgArtifacts, String> {
    if sources.len() > MAX_BATCH_DIAGRAMS {
        return Err("PlantUML batch exceeds the diagram count limit".into());
    }
    let input_bytes = sources.iter().try_fold(0_usize, |total, (_, source)| {
        total.checked_add(source.len())
    });
    if input_bytes.is_none_or(|n| n > MAX_RENDERER_INPUT_BYTES) {
        return Err("PlantUML batch exceeds the input size limit".into());
    }

    let temporary = tempfile::tempdir().map_err(|e| e.to_string())?;
    let mut paths = Vec::with_capacity(sources.len());
    for (index, (_, source)) in sources.iter().enumerate() {
        let path = temporary.path().join(format!("diagram-{index:06}.puml"));
        std::fs::write(&path, source.as_bytes()).map_err(|e| e.to_string())?;
        paths.push(path);
    }

    cmd.args(["-tsvg", "-charset", "UTF-8"]);
    cmd.args(&paths);
    cmd.current_dir(temporary.path());
    secure_renderer(&mut cmd);
    let output = run_bounded(&mut cmd, None, RENDERER_TIMEOUT)?;
    ensure_success(&output.status, &output.stderr)?;

    let mut result = Vec::with_capacity(sources.len());
    let mut total_output_bytes = 0_u64;
    for (index, (base, _)) in sources.iter().enumerate() {
        let svg = temporary.path().join(format!("diagram-{index:06}.svg"));
        if let Some(bytes) = read_bounded_svg(&svg)? {
            total_output_bytes = total_output_bytes
                .checked_add(bytes.len() as u64)
                .ok_or_else(|| "PlantUML batch output size overflow".to_string())?;
            if total_output_bytes > MAX_RENDERED_TOTAL_BYTES {
                return Err("PlantUML batch exceeds the SVG output size limit".into());
            }
            result.push((base.clone(), bytes));
        }
    }
    Ok(result)
}

fn renderer_command(jar: Option<&Path>) -> Option<Command> {
    if let Some(jar) = jar {
        let mut command = Command::new("java");
        command.arg("-jar").arg(jar);
        return Some(command);
    }

    let binary = std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|directory| directory.join(plantuml_binary_name()))
        .find(|candidate| candidate.is_file())?;
    Some(Command::new(binary))
}

#[cfg(windows)]
fn plantuml_binary_name() -> &'static str {
    "plantuml.exe"
}

#[cfg(not(windows))]
fn plantuml_binary_name() -> &'static str {
    "plantuml"
}

fn secure_renderer(command: &mut Command) {
    // SANDBOX disallows local-file and URL access while keeping the bundled
    // PlantUML themes available from the renderer's classpath.
    command.env("PLANTUML_SECURITY_PROFILE", "SANDBOX");
}

#[derive(Debug)]
struct BoundedOutput {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn run_bounded(
    command: &mut Command,
    input: Option<&[u8]>,
    timeout: Duration,
) -> Result<BoundedOutput, String> {
    if input.is_some_and(|bytes| bytes.len() > MAX_RENDERER_INPUT_BYTES) {
        return Err("PlantUML input exceeds the renderer size limit".into());
    }
    let input_file = if let Some(input) = input {
        let mut file = tempfile::tempfile().map_err(|e| e.to_string())?;
        file.write_all(input).map_err(|e| e.to_string())?;
        file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        Some(file)
    } else {
        None
    };
    let mut stdout_file = tempfile::tempfile().map_err(|e| e.to_string())?;
    let mut stderr_file = tempfile::tempfile().map_err(|e| e.to_string())?;
    configure_process_group(command);
    let mut child = command
        .stdin(input_file.map(Stdio::from).unwrap_or(Stdio::null()))
        .stdout(Stdio::from(
            stdout_file.try_clone().map_err(|e| e.to_string())?,
        ))
        .stderr(Stdio::from(
            stderr_file.try_clone().map_err(|e| e.to_string())?,
        ))
        .spawn()
        .map_err(|e| format!("could not start PlantUML renderer: {e}"))?;

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                // A wrapper may exit while leaving a renderer descendant with
                // inherited output handles. End the dedicated process group
                // before reading the bounded temporary outputs.
                terminate_process_group(&mut child);
                break status;
            }
            Ok(None) if Instant::now() < deadline => {
                if stdout_file.metadata().map_err(|e| e.to_string())?.len()
                    > MAX_RENDERER_STDOUT_BYTES as u64
                {
                    terminate_process_group(&mut child);
                    return Err("PlantUML renderer stdout exceeds the output size limit".into());
                }
                if stderr_file.metadata().map_err(|e| e.to_string())?.len()
                    > MAX_RENDERER_STDERR_BYTES as u64
                {
                    terminate_process_group(&mut child);
                    return Err("PlantUML renderer stderr exceeds the output size limit".into());
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(None) => {
                terminate_process_group(&mut child);
                return Err(format!(
                    "PlantUML renderer timed out after {} seconds",
                    timeout.as_secs_f64()
                ));
            }
            Err(error) => {
                terminate_process_group(&mut child);
                return Err(format!("could not wait for PlantUML renderer: {error}"));
            }
        }
    };

    let (stdout, stdout_total) = read_bounded_output(&mut stdout_file, MAX_RENDERER_STDOUT_BYTES)?;
    let (stderr, stderr_total) = read_bounded_output(&mut stderr_file, MAX_RENDERER_STDERR_BYTES)?;
    if stdout_total > MAX_RENDERER_STDOUT_BYTES as u64 {
        return Err("PlantUML renderer stdout exceeds the output size limit".into());
    }
    if stderr_total > MAX_RENDERER_STDERR_BYTES as u64 {
        return Err("PlantUML renderer stderr exceeds the output size limit".into());
    }

    Ok(BoundedOutput {
        status,
        stdout,
        stderr,
    })
}

fn read_bounded_output(file: &mut std::fs::File, limit: usize) -> Result<(Vec<u8>, u64), String> {
    let total = file.metadata().map_err(|e| e.to_string())?.len();
    if total > limit as u64 {
        return Err("PlantUML renderer output exceeds the output size limit".into());
    }
    file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let mut bytes = Vec::with_capacity(total as usize);
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > limit {
        return Err("PlantUML renderer output grew beyond the output size limit".into());
    }
    Ok((bytes, total))
}

#[cfg(unix)]
fn terminate_process_group(child: &mut std::process::Child) {
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(unix)]
fn configure_process_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    unsafe {
        command.pre_exec(|| {
            if libc::setpgid(0, 0) == 0 {
                Ok(())
            } else {
                Err(std::io::Error::last_os_error())
            }
        });
    }
}

#[cfg(not(unix))]
fn configure_process_group(_command: &mut Command) {}

#[cfg(not(unix))]
fn terminate_process_group(child: &mut std::process::Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn ensure_success(status: &ExitStatus, stderr: &[u8]) -> Result<(), String> {
    if status.success() {
        return Ok(());
    }
    let detail = String::from_utf8_lossy(stderr);
    if detail.is_empty() {
        Err(format!("PlantUML renderer exited with {status}"))
    } else {
        Err(format!("PlantUML renderer exited with {status}: {detail}"))
    }
}

fn read_bounded_svg(path: &Path) -> Result<Option<Vec<u8>>, String> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => metadata,
        Ok(_) => return Err("PlantUML renderer output is not a regular file".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    if metadata.len() > MAX_RENDERED_SVG_BYTES {
        return Err("PlantUML SVG exceeds the output size limit".into());
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    std::fs::File::open(path)
        .and_then(|file| {
            file.take(MAX_RENDERED_SVG_BYTES + 1)
                .read_to_end(&mut bytes)
        })
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_RENDERED_SVG_BYTES {
        return Err("PlantUML SVG grew beyond the output size limit".into());
    }
    Ok(Some(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_flattens_source_injection_and_structural_label_syntax() {
        let escaped =
            super::escape("title\n!includeurl https://example.invalid/x\n\"q\" [x] (y); <tag>&");
        assert_eq!(escaped.matches('\n').count(), 0);
        assert!(escaped.contains("!includeurl https://example.invalid/x"));
        assert!(escaped.contains("＂q＂ ［x］ （y）； &lt;tag&gt;&amp;"));
    }

    #[test]
    fn header_emits_theme_and_smetana() {
        let h = super::header("Title");
        assert!(h.contains("!theme plain"));
        assert!(h.contains("!pragma layout smetana"));
        assert!(h.contains("title Title"));
    }

    #[cfg(unix)]
    #[test]
    fn renderer_timeout_terminates_a_stalled_child() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "exec sleep 5"]);
        let started = Instant::now();
        let error = run_bounded(&mut command, None, Duration::from_millis(40)).unwrap_err();
        assert!(error.contains("timed out"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[cfg(unix)]
    #[test]
    fn batch_render_isolates_equal_basenames_and_keeps_original_keys() {
        let sources = vec![
            ("services/a/shared".to_string(), "first".to_string()),
            ("scenarios/b/shared".to_string(), "second".to_string()),
        ];
        let mut command = Command::new("/bin/sh");
        command.args([
            "-c",
            "for path do case \"$path\" in *.puml) cat \"$path\" > \"${path%.puml}.svg\";; esac; done",
            "plantuml-test",
        ]);
        let rendered = batch_render_svg_with_command(&sources, command).unwrap();
        assert_eq!(rendered.len(), 2);
        assert_eq!(rendered[0], ("services/a/shared".into(), b"first".to_vec()));
        assert_eq!(
            rendered[1],
            ("scenarios/b/shared".into(), b"second".to_vec())
        );
    }

    #[test]
    fn render_svg_returns_none_when_no_renderer() {
        let puml = b"@startuml\n[*] --> A\n@enduml\n";
        let r = super::render_svg(puml, None);
        assert!(r.is_ok());
    }
}
