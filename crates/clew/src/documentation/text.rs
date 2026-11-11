//! Text escaping shared by source-bound component/data views.
pub(super) fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
pub(super) fn mermaid_label(value: &str) -> String {
    value
        .chars()
        .map(|c| match c {
            '#' => "#35;".into(),
            '&' => "#38;".into(),
            '<' => "#60;".into(),
            '>' => "#62;".into(),
            ';' => "#59;".into(),
            '"' => "#quot;".into(),
            '\'' => "#39;".into(),
            '`' => "#96;".into(),
            '\n' | '\r' => " ".into(),
            _ => c.to_string(),
        })
        .collect()
}
