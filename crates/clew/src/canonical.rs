use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

pub fn value<T: Serialize>(input: &T) -> anyhow::Result<Value> {
    Ok(sort_value(serde_json::to_value(input)?))
}

pub fn bytes<T: Serialize>(input: &T) -> anyhow::Result<Vec<u8>> {
    Ok(serde_json::to_vec(&value(input)?)?)
}

pub fn compact<T: Serialize>(input: &T) -> anyhow::Result<String> {
    Ok(serde_json::to_string(&value(input)?)?)
}

pub fn hash<T: Serialize>(input: &T) -> anyhow::Result<String> {
    Ok(hash_bytes(&bytes(input)?))
}

pub fn hash_bytes(input: &[u8]) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(input)))
}

fn sort_value(value: Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.into_iter().map(sort_value).collect()),
        Value::Object(map) => {
            let mut entries: Vec<_> = map.into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            Value::Object(
                entries
                    .into_iter()
                    .map(|(k, v)| (k, sort_value(v)))
                    .collect(),
            )
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn canonical_objects_are_sorted_recursively() {
        let rendered =
            String::from_utf8(bytes(&json!({"z": {"b": 1, "a": 2}, "a": 0})).unwrap()).unwrap();
        assert_eq!(rendered, r#"{"a":0,"z":{"a":2,"b":1}}"#);
    }
}
