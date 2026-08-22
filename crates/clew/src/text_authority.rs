use serde_json::Value;
use unicode_normalization::UnicodeNormalization;

pub fn is_nfc(value: &str) -> bool {
    value.nfc().eq(value.chars())
}

pub fn json_strings_are_nfc(value: &Value, depth: usize) -> bool {
    if depth > 64 {
        return false;
    }
    match value {
        Value::String(value) => is_nfc(value),
        Value::Array(values) => values
            .iter()
            .all(|value| json_strings_are_nfc(value, depth + 1)),
        Value::Object(values) => values
            .iter()
            .all(|(key, value)| is_nfc(key) && json_strings_are_nfc(value, depth + 1)),
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rejects_decomposed_strings_at_any_json_depth() {
        assert!(is_nfc("é"));
        assert!(!is_nfc("e\u{301}"));
        assert!(json_strings_are_nfc(&json!({"term":["é"]}), 0));
        assert!(!json_strings_are_nfc(&json!({"term":["e\u{301}"]}), 0));
    }
}
