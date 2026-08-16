use serde::Serialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::protocol::EvidenceRef;

#[derive(Debug, Error)]
pub enum CanonicalError {
    #[error("cannot serialize canonical JSON: {0}")]
    Serialize(#[from] serde_json::Error),
}

/// Serialize using recursively sorted object keys and no insignificant space.
/// Arrays remain ordered: this is essential for compiler argument vectors.
pub fn canonical_json_bytes<T: Serialize + ?Sized>(value: &T) -> Result<Vec<u8>, CanonicalError> {
    let value = serde_json::to_value(value)?;
    Ok(serde_json::to_vec(&sort_value(value))?)
}

pub fn canonical_json<T: Serialize + ?Sized>(value: &T) -> Result<String, CanonicalError> {
    Ok(String::from_utf8(canonical_json_bytes(value)?)
        .expect("serde_json always emits valid UTF-8"))
}

pub fn sha256_digest(bytes: impl AsRef<[u8]>) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(bytes.as_ref())))
}

/// Domain-separated binary Merkle root of the canonical evidence-reference
/// leaves. Evidence references are a set and must already be sorted by the
/// enclosing protocol validator.
pub fn evidence_merkle_root(evidence: &[EvidenceRef]) -> Result<String, CanonicalError> {
    if evidence.is_empty() {
        return Ok(sha256_digest([2]));
    }
    let mut level = evidence
        .iter()
        .map(|item| {
            let canonical = canonical_json_bytes(item)?;
            let mut leaf = Vec::with_capacity(canonical.len() + 1);
            leaf.push(0);
            leaf.extend(canonical);
            Ok(Sha256::digest(leaf).to_vec())
        })
        .collect::<Result<Vec<_>, CanonicalError>>()?;
    while level.len() > 1 {
        if level.len() % 2 == 1 {
            level.push(level.last().expect("nonempty level").clone());
        }
        level = level
            .chunks_exact(2)
            .map(|pair| {
                let mut branch = Vec::with_capacity(65);
                branch.push(1);
                branch.extend(&pair[0]);
                branch.extend(&pair[1]);
                Sha256::digest(branch).to_vec()
            })
            .collect();
    }
    Ok(format!("sha256:{}", hex::encode(&level[0])))
}

fn sort_value(value: Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut entries = object.into_iter().collect::<Vec<_>>();
            entries.sort_by(|left, right| left.0.cmp(&right.0));
            Value::Object(Map::from_iter(
                entries
                    .into_iter()
                    .map(|(key, value)| (key, sort_value(value))),
            ))
        }
        Value::Array(values) => Value::Array(values.into_iter().map(sort_value).collect()),
        scalar => scalar,
    }
}

#[cfg(test)]
mod tests {
    use serde::Serialize;

    use super::*;

    #[derive(Serialize)]
    struct Ordered<'a> {
        z: &'a str,
        args: Vec<&'a str>,
        a: Nested,
    }

    #[derive(Serialize)]
    struct Nested {
        y: u8,
        x: u8,
    }

    #[test]
    fn sorts_objects_but_preserves_array_order() {
        let value = Ordered {
            z: "last",
            args: vec!["-Dsecond", "-Dfirst"],
            a: Nested { y: 2, x: 1 },
        };
        assert_eq!(
            canonical_json(&value).unwrap(),
            r#"{"a":{"x":1,"y":2},"args":["-Dsecond","-Dfirst"],"z":"last"}"#
        );
    }
}
