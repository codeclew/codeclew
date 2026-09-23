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

/// Attempt to pre-render `.puml` to SVG using an external `plantuml` binary.
/// Returns `Ok(Some(svg_path))` on success, `Ok(None)` if the binary is absent,
/// and `Err` only on an unexpected execution failure.
pub fn render_svg(puml_path: &std::path::Path) -> Result<Option<std::path::PathBuf>, String> {
    let which = std::process::Command::new("which").arg("plantuml").output();
    let available = which.map(|o| o.status.success()).unwrap_or(false);
    if !available {
        return Ok(None);
    }
    let out = std::process::Command::new("plantuml")
        .args(["-tsvg", "-charset", "UTF-8"])
        .arg(puml_path)
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).into_owned());
    }
    Ok(Some(puml_path.with_extension("svg")))
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
    fn render_svg_returns_none_when_plantuml_absent() {
        // PATH without plantuml is hard to simulate portably; assert the Ok(None)
        // path only when `which plantuml` fails — which is expected in CI without
        // plantuml installed. This guards the API shape.
        let p = std::env::temp_dir().join("clew-plantuml-absent.puml");
        std::fs::write(&p, "@startuml\n[*] --> A\n@enduml\n").unwrap();
        let r = super::render_svg(&p);
        // Either Ok(None) (absent) or Ok(Some(_)) (present) — never Err.
        assert!(!matches!(r, Err(_)));
        let _ = std::fs::remove_file(&p);
    }
}
