//! Read-only validation of historical published-answer bindings.
use super::{digest, invalid};
use crate::error::ClewError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const MAX_ANSWERS: usize = 64;

/// Owns frozen answer artifacts, separately from narrative fragment sources.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublishedAnswer {
    pub schema: String,
    pub id: String,
    pub title: String,
    pub work: String,
    pub review_run: String,
    pub source_snapshot: String,
    pub packet_digest: String,
    pub answer_digest: String,
    pub review_digest: String,
    pub provenance_digest: String,
    pub meaning_review: String,
    pub publication: String,
    pub source_context: String,
    pub artifact_hashes: BTreeMap<String, String>,
}
impl PublishedAnswer {
    pub fn route(&self) -> String {
        format!("answers/{}.html", self.id)
    }
    fn stem(&self) -> String {
        format!("answers/{}", self.id)
    }
    fn identity(&self) -> Result<String, ClewError> {
        Ok(digest(&(
            self.work.as_str(),
            self.review_run.as_str(),
            self.source_snapshot.as_str(),
            self.packet_digest.as_str(),
            self.answer_digest.as_str(),
            self.review_digest.as_str(),
            self.provenance_digest.as_str(),
        ))?[7..]
            .into())
    }
}

fn hex(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
fn hash(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|s| hex(s, 64))
}

pub(super) fn validate_entries(
    entries: &BTreeMap<String, PublishedAnswer>,
) -> Result<(), ClewError> {
    if entries.len() > MAX_ANSWERS {
        return Err(invalid("reviewed answer catalogue exceeds 64 entries"));
    }
    for (id, entry) in entries {
        if id != &entry.id
            || !hex(id, 64)
            || !hex(&entry.work, 64)
            || !hex(&entry.review_run, 32)
            || entry.schema != "codeclew-reviewed-operation-answer-publication/1.0"
            || entry.title.trim().is_empty()
            || entry.title.len() > 8192
            || entry.meaning_review != "MODEL_APPROVED"
            || entry.publication != "PUBLISHED"
            || entry.source_context != "SAVED_SNAPSHOT_NOT_REVERIFIED"
            || ![
                &entry.packet_digest,
                &entry.answer_digest,
                &entry.review_digest,
                &entry.provenance_digest,
            ]
            .into_iter()
            .all(|s| hash(s))
            || entry.source_snapshot.rsplit_once('/').is_none_or(|(d, n)| {
                !hash(d)
                    || n.parse::<u64>()
                        .ok()
                        .is_none_or(|size| n != size.to_string())
            })
            || entry.identity()? != *id
        {
            return Err(invalid("invalid closed reviewed answer binding"));
        }
        let stem = entry.stem();
        let required = [
            "answer.json",
            "packet.json",
            "audit.json",
            "meaning-review.json",
            "provenance.json",
            "md",
        ];
        if entry.artifact_hashes.len() < required.len()
            || entry.artifact_hashes.len() > required.len() + 1
            || required.iter().any(|suffix| {
                !entry
                    .artifact_hashes
                    .contains_key(&format!("{stem}.{suffix}"))
            })
            || entry.artifact_hashes.iter().any(|(p, h)| {
                !hash(h)
                    || (!required.iter().any(|s| p == &format!("{stem}.{s}"))
                        && p != &format!("{stem}.puml"))
            })
        {
            return Err(invalid(
                "reviewed answer artifact paths or hashes are invalid",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::{
        bindings::{self, Bindings},
        bytes, retained_validation,
    };
    use super::*;
    use serde_json::Value;
    #[test]
    fn empty_answer_binding_preserves_legacy_identity() {
        let work = super::super::work::api_contract_tests::endpoint_context_fixture();
        let mut binding =
            retained_validation::make_bindings(&work.checked, BTreeMap::new()).unwrap();
        bindings::compact(&mut binding);
        let original = bytes(&binding).unwrap();
        assert_eq!(binding.schema, "codeclew-documentation-bindings/1.4");
        let value: Value = serde_json::from_slice(&original).unwrap();
        assert!(value.get("reviewedAnswers").is_none());
        let restored: Bindings = serde_json::from_value(value).unwrap();
        assert_eq!(bytes(&restored).unwrap(), original);
    }
}
