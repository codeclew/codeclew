//! Presentation language never participates in compiler or source-capture identity.
#[cfg(not(test))]
use super::invalid;
#[cfg(test)]
use super::invalid;
use crate::error::ClewError;
use std::{collections::BTreeMap, sync::OnceLock};

pub(super) const RUSSIAN_LABELS: &str = include_str!("../../assets/documentation/ru.json");

/// Look up an owned display label. Callers insert source and authored values
/// separately; this function never scans a completed page or rewrites its data.
pub(super) fn label<'a>(language: &str, key: &'a str) -> &'a str {
    if language != "ru" {
        return key;
    }
    static LABELS: OnceLock<BTreeMap<String, String>> = OnceLock::new();
    LABELS
        .get_or_init(|| {
            serde_json::from_str(RUSSIAN_LABELS).expect("bundled reader labels are valid JSON")
        })
        .get(key)
        .map(String::as_str)
        .unwrap_or(key)
}

pub(super) fn validate(language: Option<&str>) -> Result<(), ClewError> {
    if language.is_some_and(|value| !matches!(value, "en" | "ru")) {
        return Err(invalid("documentation language must be en or ru"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn language_validation_and_owned_labels_preserve_caller_text() {
        assert!(validate(None).is_ok());
        assert!(validate(Some("en")).is_ok());
        assert!(validate(Some("ru")).is_ok());
        assert!(validate(Some("fr")).is_err());
        assert_eq!(label("en", "Unknown source label"), "Unknown source label");
        assert_eq!(label("ru", "Unknown source label"), "Unknown source label");
    }
}
