//! Shared, non-causal vocabulary for retained producer source structure.
//!
//! Decoding a statement does not admit its producer, prove execution order,
//! bind local values or resolve a receiver. Consumers admit the evidence
//! envelope separately and keep their own qualified feature boundaries.
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StructureProducer {
    Javac,
    KotlinPsi,
    Roslyn,
    TypeScript,
    RustSyntax,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct StructureContract {
    pub(crate) producer: StructureProducer,
}

impl StructureContract {
    /// Schema and authority must agree, and both ordered events and explicit
    /// boundaries must be present. This grants source structure only.
    pub(crate) fn admit(documentation: &Value) -> Option<Self> {
        let contract = Self::identify(documentation)?;
        (documentation["events"].is_array() && documentation["boundaries"].is_array())
            .then_some(contract)
    }

    /// Identify a known envelope even when its body is malformed, so existing
    /// readers can report a precise unavailable marker instead of omitting it.
    pub(crate) fn identify(documentation: &Value) -> Option<Self> {
        let producer = match (
            documentation["schema"].as_str(),
            documentation["authority"].as_str(),
        ) {
            (Some("codeclew-java-documentation-flow/1.0"), Some("JAVAC_SOURCE_STRUCTURE")) => {
                StructureProducer::Javac
            }
            (
                Some("codeclew-kotlin-documentation-flow/1.0"),
                Some("KOTLIN_PSI_WITH_K2_CALL_TARGETS"),
            ) => StructureProducer::KotlinPsi,
            (Some("codeclew-csharp-documentation-flow/1.0"), Some("ROSLYN_SOURCE_STRUCTURE")) => {
                StructureProducer::Roslyn
            }
            (
                Some("codeclew-typescript-documentation-flow/1.0"),
                Some("TYPESCRIPT_COMPILER_SOURCE_STRUCTURE"),
            ) => StructureProducer::TypeScript,
            (Some("codeclew-rust-documentation-flow/1.0"), Some("RUST_SYN_SOURCE_STRUCTURE")) => {
                StructureProducer::RustSyntax
            }
            _ => return None,
        };
        Some(Self { producer })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InvocationKind {
    Call,
    Construct,
}

impl InvocationKind {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Call => "CALL",
            Self::Construct => "CONSTRUCT",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Invocation<'a> {
    pub(crate) kind: InvocationKind,
    pub(crate) target: Option<&'a str>,
    pub(crate) compiler_exact: bool,
    pub(crate) unresolved: bool,
    pub(crate) has_http_metadata: bool,
}

impl<'a> Invocation<'a> {
    pub(crate) fn exact_target(self) -> Option<&'a str> {
        (self.compiler_exact && !self.unresolved)
            .then_some(self.target)
            .flatten()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SourceStatement<'a> {
    If {
        condition: Option<&'a str>,
        subject_present: bool,
    },
    Else {
        condition: Option<&'a str>,
        condition_present: bool,
        subject_present: bool,
    },
    ElseIf(Option<&'a str>),
    Invocation(Invocation<'a>),
    Return,
    Local,
    Expression,
    End,
    Boundary(Option<&'a str>),
    Unsupported(&'a str),
    Missing,
}

impl<'a> SourceStatement<'a> {
    /// Preserve malformed/unknown metadata instead of repairing a producer's
    /// structure. Exact expression bytes remain in the separately bound span.
    pub(crate) fn decode(event: &'a Value) -> Self {
        match event["kind"].as_str().unwrap_or("") {
            "IF" => Self::If {
                condition: event["condition"].as_str(),
                subject_present: event.get("subject").is_some(),
            },
            "ELSE" => Self::Else {
                condition: event["condition"].as_str(),
                condition_present: event.get("condition").is_some(),
                subject_present: event.get("subject").is_some(),
            },
            "ELSEIF" => Self::ElseIf(event["condition"].as_str()),
            kind @ ("CALL" | "CONSTRUCT") => Self::Invocation(Invocation {
                kind: if kind == "CALL" {
                    InvocationKind::Call
                } else {
                    InvocationKind::Construct
                },
                target: event["target"].as_str().filter(|target| !target.is_empty()),
                compiler_exact: event["resolution"] == "COMPILER_EXACT",
                unresolved: event["targetStatus"] == "UNRESOLVED",
                has_http_metadata: event["http"]
                    .as_object()
                    .is_some_and(|http| !http.is_empty()),
            }),
            "RETURN" => Self::Return,
            "LOCAL" => Self::Local,
            "STATEMENT" => Self::Expression,
            "END" => Self::End,
            "BOUNDARY" => Self::Boundary(event["code"].as_str()),
            "" => Self::Missing,
            unknown => Self::Unsupported(unknown),
        }
    }

    pub(crate) fn label(self) -> &'a str {
        match self {
            Self::If { .. } => "IF",
            Self::Else { .. } => "ELSE",
            Self::ElseIf(_) => "ELSEIF",
            Self::Invocation(call) => call.kind.label(),
            Self::Return => "RETURN",
            Self::Local => "LOCAL",
            Self::Expression => "STATEMENT",
            Self::End => "END",
            Self::Boundary(_) => "BOUNDARY",
            Self::Unsupported(kind) => kind,
            Self::Missing => "",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn producer_admission_keeps_authority_and_boundaries_distinct() {
        let producers = [
            (
                "codeclew-java-documentation-flow/1.0",
                "JAVAC_SOURCE_STRUCTURE",
                StructureProducer::Javac,
            ),
            (
                "codeclew-kotlin-documentation-flow/1.0",
                "KOTLIN_PSI_WITH_K2_CALL_TARGETS",
                StructureProducer::KotlinPsi,
            ),
            (
                "codeclew-csharp-documentation-flow/1.0",
                "ROSLYN_SOURCE_STRUCTURE",
                StructureProducer::Roslyn,
            ),
            (
                "codeclew-typescript-documentation-flow/1.0",
                "TYPESCRIPT_COMPILER_SOURCE_STRUCTURE",
                StructureProducer::TypeScript,
            ),
            (
                "codeclew-rust-documentation-flow/1.0",
                "RUST_SYN_SOURCE_STRUCTURE",
                StructureProducer::RustSyntax,
            ),
        ];
        for (schema, authority, producer) in producers {
            let mut doc =
                json!({"schema":schema,"authority":authority,"events":[],"boundaries":[]});
            assert_eq!(StructureContract::admit(&doc).unwrap().producer, producer);
            doc["boundaries"] = Value::Null;
            assert!(StructureContract::admit(&doc).is_none());
            doc["boundaries"] = json!([]);
            doc["schema"] = json!("unknown/1.0");
            assert!(StructureContract::admit(&doc).is_none());
        }
        let forged = json!({"schema":"codeclew-csharp-documentation-flow/1.0",
            "authority":"JAVAC_SOURCE_STRUCTURE","events":[],"boundaries":[]});
        assert!(StructureContract::admit(&forged).is_none());
    }

    #[test]
    fn unsupported_and_malformed_structure_is_not_repaired() {
        let event = json!({"kind":"ELSE","condition":null,"subject":null});
        assert_eq!(
            SourceStatement::decode(&event),
            SourceStatement::Else {
                condition: None,
                condition_present: true,
                subject_present: true,
            }
        );
        let unknown = json!({"kind":"AWAIT_DYNAMIC"});
        assert_eq!(
            SourceStatement::decode(&unknown),
            SourceStatement::Unsupported("AWAIT_DYNAMIC")
        );
        assert_eq!(
            SourceStatement::decode(&json!({"kind":7})),
            SourceStatement::Missing
        );
    }

    #[test]
    fn source_call_text_alone_never_grants_an_exact_target() {
        for event in [
            json!({"kind":"CALL","target":"helper"}),
            json!({"kind":"CALL","target":"helper","resolution":"SYNTAX_UNRESOLVED"}),
            json!({"kind":"CALL","target":"helper","resolution":"COMPILER_EXACT","targetStatus":"UNRESOLVED"}),
            json!({"kind":"CALL","resolution":"COMPILER_EXACT"}),
        ] {
            let SourceStatement::Invocation(call) = SourceStatement::decode(&event) else {
                panic!("invocation")
            };
            assert!(call.exact_target().is_none());
        }
        let event =
            json!({"kind":"CONSTRUCT","target":"constructor","resolution":"COMPILER_EXACT"});
        let SourceStatement::Invocation(call) = SourceStatement::decode(&event) else {
            panic!("invocation")
        };
        assert_eq!(call.exact_target(), Some("constructor"));
        assert_eq!(call.kind, InvocationKind::Construct);
    }
}
