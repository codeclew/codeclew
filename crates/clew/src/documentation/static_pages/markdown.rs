//! An authored body is data for the shared parser, never executable MDX.
pub(super) fn mdx(text: &str) -> String {
    let literal = serde_json::to_string(text)
        .expect("text serializes")
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029");
    format!("<ClewMarkdown text={{{literal}}} />")
}

#[cfg(test)]
mod tests {
    #[test]
    fn authored_mdx_and_html_stay_literal_data_and_round_trip_exactly() {
        let text = "import code from 'x'\n<script>{unsafe()}</script>\n| A | B |\n| --- | --- |\n| `a<b` | [source](https://example.test?a=1&b=2) |";
        let body = super::mdx(text);
        assert!(!body.contains("<script>"));
        let literal = body
            .strip_prefix("<ClewMarkdown text={")
            .unwrap()
            .strip_suffix("} />")
            .unwrap();
        assert_eq!(serde_json::from_str::<String>(literal).unwrap(), text);
    }
}
