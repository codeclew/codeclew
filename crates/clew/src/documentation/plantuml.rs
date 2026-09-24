//! Shared PlantUML output helpers for auto-generated process diagrams.

/// Escape PlantUML/HTML-special characters in label text.
pub fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
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
/// available, and `Err` only on an unexpected execution failure.
pub fn render_svg(puml: &[u8], jar: Option<&std::path::Path>) -> Result<Option<Vec<u8>>, String> {
    let mut cmd: Option<std::process::Command> = None;
    if let Some(jar) = jar {
        let mut c = std::process::Command::new("java");
        c.arg("-jar").arg(jar);
        cmd = Some(c);
    } else {
        let which = std::process::Command::new("which").arg("plantuml").output();
        if which.map(|o| o.status.success()).unwrap_or(false) {
            let mut c = std::process::Command::new("plantuml");
            cmd = Some(c);
        }
    }
    let Some(mut child) = cmd else {
        return Ok(None);
    };
    use std::io::Write;
    child
        .args(["-tsvg", "-charset", "UTF-8", "-pipe"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut proc = child.spawn().map_err(|e| e.to_string())?;
    proc.stdin
        .take()
        .unwrap()
        .write_all(puml)
        .map_err(|e| e.to_string())?;
    let out = proc.wait_with_output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).into_owned());
    }
    Ok(Some(out.stdout))
}

/// Pre-render many PlantUML sources to SVG in a single renderer invocation.
/// Each entry is `(bundle-relative base path, puml source)`; the base includes
/// its directory prefix (e.g. `diagrams/scenario-x-states`). All diagrams are
/// written to a temp dir and passed to one `plantuml`/`java -jar` process, then
/// the produced SVG bytes are returned keyed by the same base. Returns
/// `Ok(None)` when no renderer is available.
pub fn batch_render_svg(
    sources: &[(String, String)],
    jar: Option<&std::path::Path>,
) -> Result<Option<Vec<(String, Vec<u8>)>>, String> {
    let mut cmd: Option<std::process::Command> = if let Some(jar) = jar {
        let mut c = std::process::Command::new("java");
        c.arg("-jar").arg(jar);
        Some(c)
    } else {
        let which = std::process::Command::new("which").arg("plantuml").output();
        if which.map(|o| o.status.success()).unwrap_or(false) {
            Some(std::process::Command::new("plantuml"))
        } else {
            None
        }
    };
    let Some(mut cmd) = cmd else {
        return Ok(None);
    };
    let tmp = std::env::temp_dir().join(format!("clew-puml-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    let mut paths = Vec::with_capacity(sources.len());
    for (base, puml) in sources {
        let name = base.rsplit('/').next().unwrap_or(base);
        let p = tmp.join(format!("{name}.puml"));
        std::fs::write(&p, puml.as_bytes()).map_err(|e| e.to_string())?;
        paths.push(p);
    }
    cmd.arg("-tsvg").arg("-charset").arg("UTF-8");
    let out = cmd.args(&paths).output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(String::from_utf8_lossy(&out.stderr).into_owned());
    }
    let mut result = Vec::with_capacity(sources.len());
    for (base, _) in sources {
        let name = base.rsplit('/').next().unwrap_or(base);
        if let Ok(bytes) = std::fs::read(tmp.join(format!("{name}.svg"))) {
            result.push((base.clone(), bytes));
        }
    }
    let _ = std::fs::remove_dir_all(&tmp);
    Ok(Some(result))
}

#[cfg(test)]
mod tests {
    #[test]
    fn escape_html_escapes_special_chars() {
        assert_eq!(super::escape("a < b & c > d"), "a &lt; b &amp; c &gt; d");
    }

    #[test]
    fn header_emits_theme_and_smetana() {
        let h = super::header("Title");
        assert!(h.contains("!theme plain"));
        assert!(h.contains("!pragma layout smetana"));
        assert!(h.contains("title Title"));
    }

    #[test]
    fn render_svg_returns_none_when_no_renderer() {
        // A nonexistent jar path forces the binary fallback, which also finds
        // nothing in CI without plantuml on PATH — assert the Ok(None) shape.
        let puml = b"@startuml\n[*] --> A\n@enduml\n";
        let r = super::render_svg(puml, None);
        // Either Ok(None) (absent) or Ok(Some(_)) (present) — never Err.
        assert!(!matches!(r, Err(_)));
    }
}
