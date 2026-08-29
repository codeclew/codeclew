use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fmt;

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

/// Parse caller-authored JSON without allowing duplicate object keys to be
/// silently replaced before canonicalization.
pub fn parse_json_strict(input: &[u8]) -> serde_json::Result<Value> {
    let mut deserializer = serde_json::Deserializer::from_slice(input);
    let StrictValue(value) = StrictValue::deserialize(&mut deserializer)?;
    deserializer.end()?;
    Ok(value)
}

struct StrictValue(Value);

impl<'de> Deserialize<'de> for StrictValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(StrictValueVisitor)
    }
}

struct StrictValueVisitor;

impl<'de> Visitor<'de> for StrictValueVisitor {
    type Value = StrictValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON value without duplicate object keys")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::Bool(value)))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::Number(value.into())))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::Number(value.into())))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        serde_json::Number::from_f64(value)
            .map(Value::Number)
            .map(StrictValue)
            .ok_or_else(|| E::custom("JSON number is not finite"))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::String(value.to_owned())))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::String(value)))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::Null))
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::Null))
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        StrictValue::deserialize(deserializer)
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(StrictValue(value)) = sequence.next_element()? {
            values.push(value);
        }
        Ok(StrictValue(Value::Array(values)))
    }

    fn visit_map<A>(self, mut object: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut values = serde_json::Map::new();
        while let Some(key) = object.next_key::<String>()? {
            if values.contains_key(&key) {
                return Err(de::Error::custom("duplicate JSON object key"));
            }
            let StrictValue(value) = object.next_value()?;
            values.insert(key, value);
        }
        Ok(StrictValue(Value::Object(values)))
    }
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

    #[test]
    fn strict_json_parser_rejects_duplicate_keys_at_every_depth() {
        assert!(parse_json_strict(br#"{"a":1,"a":2}"#).is_err());
        assert!(parse_json_strict(br#"{"a":{"b":1,"b":2}}"#).is_err());
        assert_eq!(
            parse_json_strict(b" {\"a\":[true,null,1.5]} \n").unwrap(),
            serde_json::json!({"a":[true,null,1.5]})
        );
    }
}
