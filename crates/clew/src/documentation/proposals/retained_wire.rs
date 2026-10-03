//! Target-specific closed wire shapes preserve legacy retained edit identities.
use super::*;
#[derive(Clone, Serialize, Deserialize)]
#[serde(
    tag = "target",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum Wire {
    OperationTitle {
        kind: RetainedKind,
        id: String,
        record_digest: String,
        expected_old_value: String,
        replacement: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fragment_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        author: Option<String>,
    },
    ExplanationText {
        kind: RetainedKind,
        id: String,
        record_digest: String,
        expected_old_value: String,
        replacement: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fragment_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        author: Option<String>,
    },
    ExplanationContext {
        kind: RetainedKind,
        id: String,
        record_digest: String,
        fragment_id: String,
        expected_paragraph_digest: String,
        expected_context_digest: String,
        context_editor: String,
        source_references: Vec<String>,
        dependency_references: Vec<String>,
        anchors: Vec<ContextAnchor>,
    },
}
impl<'de> Deserialize<'de> for RetainedEdit {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(match Wire::deserialize(deserializer)? {
            Wire::OperationTitle {
                kind,
                id,
                record_digest,
                expected_old_value,
                replacement,
                fragment_id,
                author,
            } => Self {
                kind,
                id,
                record_digest,
                target: RetainedTarget::OperationTitle,
                expected_old_value,
                replacement,
                fragment_id,
                author,
                context: None,
            },
            Wire::ExplanationText {
                kind,
                id,
                record_digest,
                expected_old_value,
                replacement,
                fragment_id,
                author,
            } => Self {
                kind,
                id,
                record_digest,
                target: RetainedTarget::ExplanationText,
                expected_old_value,
                replacement,
                fragment_id,
                author,
                context: None,
            },
            Wire::ExplanationContext {
                kind,
                id,
                record_digest,
                fragment_id,
                expected_paragraph_digest,
                expected_context_digest,
                context_editor,
                source_references,
                dependency_references,
                anchors,
            } => Self {
                kind,
                id,
                record_digest,
                target: RetainedTarget::ExplanationContext,
                expected_old_value: String::new(),
                replacement: String::new(),
                fragment_id: Some(fragment_id),
                author: None,
                context: Some(ContextInstruction {
                    expected_paragraph_digest,
                    expected_context_digest,
                    context_editor,
                    source_references,
                    dependency_references,
                    anchors,
                }),
            },
        })
    }
}
impl Serialize for RetainedEdit {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let value = match self.target {
            RetainedTarget::OperationTitle => {
                if self.context.is_some() {
                    return Err(serde::ser::Error::custom(
                        "text/title edit cannot carry context migration",
                    ));
                }
                Wire::OperationTitle {
                    kind: self.kind.clone(),
                    id: self.id.clone(),
                    record_digest: self.record_digest.clone(),
                    expected_old_value: self.expected_old_value.clone(),
                    replacement: self.replacement.clone(),
                    fragment_id: self.fragment_id.clone(),
                    author: self.author.clone(),
                }
            }
            RetainedTarget::ExplanationText => {
                if self.context.is_some() {
                    return Err(serde::ser::Error::custom(
                        "text/title edit cannot carry context migration",
                    ));
                }
                Wire::ExplanationText {
                    kind: self.kind.clone(),
                    id: self.id.clone(),
                    record_digest: self.record_digest.clone(),
                    expected_old_value: self.expected_old_value.clone(),
                    replacement: self.replacement.clone(),
                    fragment_id: self.fragment_id.clone(),
                    author: self.author.clone(),
                }
            }
            RetainedTarget::ExplanationContext => {
                if self.author.is_some()
                    || !self.expected_old_value.is_empty()
                    || !self.replacement.is_empty()
                {
                    return Err(serde::ser::Error::custom(
                        "context migration cannot carry a text edit",
                    ));
                }
                let c = self
                    .context
                    .as_ref()
                    .ok_or_else(|| serde::ser::Error::custom("context instruction missing"))?;
                Wire::ExplanationContext {
                    kind: self.kind.clone(),
                    id: self.id.clone(),
                    record_digest: self.record_digest.clone(),
                    fragment_id: self
                        .fragment_id
                        .clone()
                        .ok_or_else(|| serde::ser::Error::custom("context fragment missing"))?,
                    expected_paragraph_digest: c.expected_paragraph_digest.clone(),
                    expected_context_digest: c.expected_context_digest.clone(),
                    context_editor: c.context_editor.clone(),
                    source_references: c.source_references.clone(),
                    dependency_references: c.dependency_references.clone(),
                    anchors: c.anchors.clone(),
                }
            }
        };
        value.serialize(serializer)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_wire_identity_and_closed_context_payloads() {
        let old = json!({"kind":"RETAINED_OPERATION","id":"op","recordDigest":"sha256:record","target":"explanationText","fragmentId":"p","author":"Fixture author","expectedOldValue":"old","replacement":"new"});
        let parsed: RetainedEdit = serde_json::from_value(old.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), old);
        let context = json!({"kind":"RETAINED_OPERATION","id":"op","recordDigest":"sha256:record","target":"explanationContext","fragmentId":"p","expectedParagraphDigest":"sha256:p","expectedContextDigest":"sha256:c","contextEditor":"Fixture editor","sourceReferences":["s1"],"dependencyReferences":["d1"],"anchors":[{"eventId":"e","expectedEventDigest":"sha256:e"}]});
        assert!(serde_json::from_value::<RetainedEdit>(context.clone()).is_ok());
        for key in ["author", "expectedOldValue", "replacement", "unknown"] {
            let mut bad = context.clone();
            bad[key] = json!("forbidden");
            assert!(
                serde_json::from_value::<RetainedEdit>(bad).is_err(),
                "{key}"
            );
        }
        let mut bad = old;
        bad["contextEditor"] = json!("forbidden");
        assert!(serde_json::from_value::<RetainedEdit>(bad).is_err());
    }
}
