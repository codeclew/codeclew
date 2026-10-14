//! Shared standalone operation document presentation; independent of evidence contracts.
pub(super) const STYLE: &str = include_str!("../../assets/documentation/operation-reader.css");
pub(super) fn citations(title: &str, links: &str) -> String {
    format!(
        "<details class=\"citations\"><summary>{}</summary><span class=\"citation-list\">{links}</span></details>",
        super::render::escape(title)
    )
}
