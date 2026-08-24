#[derive(Debug)]
pub struct Formatter;

impl Formatter {
    pub fn format_message(&self, name: &str) -> String {
        super::format_message(name)
    }
}

#[cfg(feature = "experimental")]
pub fn experimental_message() -> &'static str {
    "experimental"
}
