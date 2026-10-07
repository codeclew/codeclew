//! Producer admission for shared retained structure and exact-call navigation.
//! Portable signature grammar does not transfer Java compiler authority.
use super::super::model::{CallableProjection, ProjectionKind};
use super::{CallableKey, Context, ProjectedCallable, gap};
use crate::documentation::source_statement::{StructureContract, StructureProducer};
use crate::documentation::{digest, invalid, model::Observation};
use crate::error::ClewError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CallNavigation {
    JavaBehavioral,
    RetainedCompilerSites,
    UnresolvedSyntax,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct CallableCapabilities {
    pub(super) producer: StructureProducer,
    pub(super) source_structure: Option<StructureContract>,
    pub(super) call_navigation: CallNavigation,
}

/// Declaration authority and body structure are admitted independently.
/// Structure never grants Java's behavior/data consumers to other producers.
pub(super) fn capabilities(owner: &Observation) -> Option<CallableCapabilities> {
    let (producer, call_navigation) = if super::java::compiler(owner) {
        (StructureProducer::Javac, CallNavigation::JavaBehavioral)
    } else if csharp_admitted(owner) {
        (
            StructureProducer::Roslyn,
            CallNavigation::RetainedCompilerSites,
        )
    } else if kotlin_admitted(owner) {
        (
            StructureProducer::KotlinPsi,
            CallNavigation::RetainedCompilerSites,
        )
    } else if typescript_admitted(owner) {
        (
            StructureProducer::TypeScript,
            CallNavigation::RetainedCompilerSites,
        )
    } else if rust_admitted(owner) {
        (
            StructureProducer::RustSyntax,
            CallNavigation::UnresolvedSyntax,
        )
    } else {
        return None;
    };
    let source_structure = StructureContract::admit(&owner.normalized["documentation"])
        .filter(|contract| contract.producer == producer);
    Some(CallableCapabilities {
        producer,
        source_structure,
        call_navigation,
    })
}

pub(super) fn rust_admitted(owner: &Observation) -> bool {
    let n = &owner.normalized;
    owner.kind == "SYMBOL"
        && n["schema"] == "codeclew-rust-syntax-fact/1.2"
        && n["kind"] == "DECLARATION"
        && n["sourceFactKind"] == "declaration"
        && n["resolution"] == "SYNTAX_EXACT"
        && n["symbolIdentity"] == owner.symbol
        && owner.symbol.starts_with("rust-syntax:")
        && n["scope"]
            .as_str()
            .is_some_and(|scope| scope.starts_with("cargo:"))
        && matches!(
            n["declarationKind"].as_str(),
            Some(
                "function"
                    | "impl-method"
                    | "trait-method"
                    | "struct"
                    | "enum"
                    | "trait"
                    | "type-alias"
                    | "module"
                    | "const"
                    | "static"
            )
        )
}

pub(super) fn typescript_admitted(owner: &Observation) -> bool {
    let n = &owner.normalized;
    owner.kind == "SYMBOL"
        && n["schema"] == crate::typescript_adapter_v2::TYPESCRIPT_FACT_SCHEMA
        && n["kind"] == "DECLARATION"
        && matches!(n["declarationKind"].as_str(), Some("FUNCTION" | "METHOD"))
        && n["resolution"] == "COMPILER_RESOLVED"
        && n["symbolIdentity"] == owner.symbol
        && n["declarationIdentity"] == owner.symbol
        && owner.symbol.starts_with("ts:")
        && n["scope"]
            .as_str()
            .is_some_and(|s| s.starts_with("tsconfig:"))
}

/// Retained input/return display only; a signature does not bind argument values.
pub(in crate::documentation::static_pages) fn declared_signature<'a>(
    owner: &'a Observation,
    sources: &std::collections::BTreeMap<String, crate::documentation::model::Source>,
) -> Option<&'a str> {
    if !(typescript_admitted(owner) || rust_admitted(owner)) {
        return None;
    }
    let signature = owner.normalized["signature"]
        .as_str()
        .filter(|text| !text.is_empty() && text.len() <= 4096)?;
    if rust_admitted(owner) {
        let site = &owner.normalized["outlineOwnerSource"];
        let source = sources.get(site["sourceId"].as_str()?)?;
        let owner_start = usize::try_from(site["byteStart"].as_u64()?).ok()?;
        let owner_end = usize::try_from(site["byteEnd"].as_u64()?).ok()?;
        if site["sourceStatus"] != "SOURCE_RETAINED"
            || site["sourceId"] != source.id
            || owner.source_ids.as_slice() != [source.id.clone()]
            || source.service != owner.service
            || site["file"] != source.file
            || site["sourceDigest"] != source.text_digest
            || site["evidenceDigest"] != source.evidence_digest
            || source.text_digest != crate::canonical::hash_bytes(source.text.as_bytes())
            || owner_end.checked_sub(owner_start) != Some(source.text.len())
        {
            return None;
        }
        let start = usize::try_from(owner.normalized["signatureStart"].as_u64()?)
            .ok()?
            .checked_sub(owner_start)?;
        let end = usize::try_from(owner.normalized["signatureEnd"].as_u64()?)
            .ok()?
            .checked_sub(owner_start)?;
        if source.text.get(start..end) != Some(signature) {
            return None;
        }
    }
    Some(signature)
}

pub(super) fn csharp_candidate(owner: &Observation) -> bool {
    owner.normalized["schema"]
        .as_str()
        .is_some_and(|schema| schema.starts_with("codeclew-csharp-compiler-fact/"))
}

pub(super) fn csharp_admitted(owner: &Observation) -> bool {
    let n = &owner.normalized;
    n["schema"] == crate::csharp_adapter_v2::CSHARP_FACT_SCHEMA
        && owner.kind == "SYMBOL"
        && n["kind"] == "DECLARATION"
        && n["declarationKind"] == "METHOD"
        && n["resolution"] == "COMPILER_EXACT"
        && n["symbolIdentity"] == owner.symbol
        && n["scope"].as_str().is_some_and(|s| !s.is_empty())
        && n["csharpIdentity"]
            .as_str()
            .is_some_and(|s| s.starts_with("csharp:M:"))
        && n["name"]
            .as_str()
            .zip(n["jvmDescriptor"].as_str())
            .is_some_and(|(name, descriptor)| {
                n["ownerIdentity"].as_str().is_some_and(|class| {
                    class.starts_with("class:")
                        && (owner.symbol == format!("method:{class}#{name}{descriptor}")
                            || n["csharpIdentity"].as_str().is_some_and(|id| {
                                owner.symbol
                                    == format!(
                                        "method:{class}#{name}{descriptor}@{}",
                                        &crate::canonical::hash_bytes(
                                            id.strip_prefix("csharp:").unwrap_or(id).as_bytes()
                                        )[7..19]
                                    )
                            }))
                })
            })
}

pub(in crate::documentation::static_pages) fn admitted(owner: &Observation) -> bool {
    capabilities(owner).is_some_and(|capabilities| {
        capabilities.call_navigation == CallNavigation::RetainedCompilerSites
    })
}

fn kotlin_admitted(owner: &Observation) -> bool {
    let n = &owner.normalized;
    owner.kind == "SYMBOL"
        && n["schema"] == "declaration-descriptor/0.1"
        && n["declarationKind"] == "FUNCTION"
        && n["resolution"] == "PROVEN"
        && n["provider"] == "K2_FIR"
        && n["sourceProvenance"] == "COMPILER_UTF16_RANGE_TO_UTF8_BYTES"
        && n["compilerAuthority"] == "fir-facts-extractor/0.6"
        && n["symbolIdentity"] == owner.symbol
        && crate::semantic_validation::validate_kotlin_full_symbol_identity(&owner.symbol).is_ok()
}

pub(in crate::documentation::static_pages) fn body_envelope(owner: &Observation) -> bool {
    capabilities(owner).is_some_and(|capabilities| {
        capabilities.call_navigation == CallNavigation::RetainedCompilerSites
            && capabilities.source_structure.is_some()
    })
}

pub(super) fn project_declaration(
    context: &mut Context<'_>,
    id: &str,
) -> Result<ProjectedCallable, ClewError> {
    let owner = context
        .evidence
        .observations
        .get(id)
        .ok_or_else(|| invalid("compiler declaration is missing"))?
        .clone();
    if !(csharp_admitted(&owner) || typescript_admitted(&owner) || rust_admitted(&owner))
        || owner.id != id
        || owner.service != context.evidence.service
        || owner.digest != digest(&owner.normalized)?
    {
        return Err(invalid(
            "Selected declaration lacks admitted retained producer authority",
        ));
    }
    let scope = owner.normalized["scope"].as_str().unwrap();
    if context
        .evidence
        .observations
        .values()
        .filter(|candidate| {
            candidate.kind == "SYMBOL"
                && candidate.symbol == owner.symbol
                && candidate.normalized["scope"] == scope
        })
        .count()
        != 1
    {
        return Err(invalid(
            "Compiler declaration identity is ambiguous within its compilation scope",
        ));
    }
    let source = match owner.source_ids.as_slice() {
        [id] => context
            .evidence
            .sources
            .get(id)
            .filter(|source| {
                source.id == *id
                    && context.valid_source(source)
                    && crate::documentation::store::relative(&source.file).is_ok()
            })
            .cloned(),
        _ => None,
    }
    .ok_or_else(|| {
        invalid("Compiler declaration requires one valid retained compiler-bound source span")
    })?;
    let citation = context.citation(&source, 0, source.text.len());
    context.retain(&owner);
    let mut projection = CallableProjection {
        declaration_id: owner.id.clone(),
        symbol: owner.symbol.clone(),
        authority: if rust_admitted(&owner) {
            "SYNTAX_DECLARATION"
        } else {
            "COMPILER_DECLARATION"
        }
        .into(),
        citation_id: Some(citation.clone()),
        control_flow: None,
        source_outline: None,
        retained_call_sites: None,
        steps: vec![],
        state: vec![],
        gaps: vec![
            gap(
                "SOURCE_BEHAVIOR_PROJECTION_UNAVAILABLE",
                if rust_admitted(&owner) {
                    "Retained syntax does not establish compiler resolution, statement effects or runtime behavior."
                } else {
                    "Retained source structure and compiler call targets do not establish statement effects or runtime behavior."
                },
                Some(citation.clone()),
            ),
            gap(
                "SOURCE_CALL_GRAPH_UNAVAILABLE",
                "Source-call body expansion is unavailable for this declaration page.",
                Some(citation.clone()),
            ),
            gap(
                "DATA_STATE_UNAVAILABLE",
                "Data-flow and state transformations are unavailable for this declaration page.",
                Some(citation),
            ),
        ],
    };
    super::outline::attach(context, &owner, scope, &mut projection)?;
    if !rust_admitted(&owner) {
        super::call_sites::attach(context, &owner, scope, &mut projection)?;
    }
    Ok(ProjectedCallable {
        key: CallableKey {
            service: owner.service,
            scope: scope.into(),
            symbol: owner.symbol,
        },
        projection,
        kind: ProjectionKind::DeclarationOnly,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn owner(symbol: &str, normalized: serde_json::Value) -> Observation {
        Observation {
            id: "owner".into(),
            kind: "SYMBOL".into(),
            service: "sample".into(),
            symbol: symbol.into(),
            digest: digest(&normalized).unwrap(),
            normalized,
            source_ids: vec![],
        }
    }

    #[test]
    fn common_capabilities_do_not_transfer_behavior_or_foreign_structure() {
        let java = "method:class:example.Pipeline#run()V";
        let kotlin = "callable:example/Pipeline.run#jvm:()V";
        let csharp = "method:class:Example.Pipeline#Run()V";
        let typescript = "ts:src/probe.ts#function:run@0-32";
        let rust = "rust-syntax:src/lib.rs#function:run@0-32";
        let producers = [
            owner(
                java,
                json!({"schema":"codeclew-java-compiler-fact/1.0",
                "declarationKind":"METHOD","symbolIdentity":java,"scope":"main"}),
            ),
            owner(
                kotlin,
                json!({"schema":"declaration-descriptor/0.1",
                "declarationKind":"FUNCTION","symbolIdentity":kotlin,"scope":"main",
                "provider":"K2_FIR","resolution":"PROVEN",
                "sourceProvenance":"COMPILER_UTF16_RANGE_TO_UTF8_BYTES",
                "compilerAuthority":"fir-facts-extractor/0.6"}),
            ),
            owner(
                csharp,
                json!({"schema":crate::csharp_adapter_v2::CSHARP_FACT_SCHEMA,
                "kind":"DECLARATION","declarationKind":"METHOD","symbolIdentity":csharp,
                "scope":"main","resolution":"COMPILER_EXACT","csharpIdentity":"csharp:M:Example.Pipeline.Run",
                "name":"Run","jvmDescriptor":"()V","ownerIdentity":"class:Example.Pipeline"}),
            ),
            owner(
                typescript,
                json!({"schema":crate::typescript_adapter_v2::TYPESCRIPT_FACT_SCHEMA,
                "kind":"DECLARATION","declarationKind":"FUNCTION","resolution":"COMPILER_RESOLVED",
                "symbolIdentity":typescript,"declarationIdentity":typescript,"scope":"tsconfig:tsconfig.json"}),
            ),
            owner(
                rust,
                json!({"schema":"codeclew-rust-syntax-fact/1.2", "kind":"DECLARATION", "sourceFactKind":"declaration",
                "declarationKind":"function", "resolution":"SYNTAX_EXACT", "symbolIdentity":rust, "scope":"cargo:Cargo.toml#probe#lib#probe"}),
            ),
        ];
        let docs = [
            json!({"schema":"codeclew-java-documentation-flow/1.0","authority":"JAVAC_SOURCE_STRUCTURE","events":[],"boundaries":[]}),
            json!({"schema":"codeclew-kotlin-documentation-flow/1.0","authority":"KOTLIN_PSI_WITH_K2_CALL_TARGETS","events":[],"boundaries":[]}),
            json!({"schema":"codeclew-csharp-documentation-flow/1.0","authority":"ROSLYN_SOURCE_STRUCTURE","events":[],"boundaries":[]}),
            json!({"schema":"codeclew-typescript-documentation-flow/1.0","authority":"TYPESCRIPT_COMPILER_SOURCE_STRUCTURE","events":[],"boundaries":[]}),
            json!({"schema":"codeclew-rust-documentation-flow/1.0","authority":"RUST_SYN_SOURCE_STRUCTURE","events":[],"boundaries":[]}),
        ];
        for (index, mut owner) in producers.into_iter().enumerate() {
            for (body_index, body) in docs.iter().enumerate() {
                owner.normalized["documentation"] = body.clone();
                let c = capabilities(&owner).unwrap();
                assert_eq!(c.source_structure.is_some(), index == body_index);
                assert_eq!(
                    c.call_navigation == CallNavigation::JavaBehavioral,
                    index == 0
                );
                assert_eq!(admitted(&owner), (1..4).contains(&index));
                assert_eq!(
                    body_envelope(&owner),
                    (1..4).contains(&index) && index == body_index
                );
                assert_eq!(
                    c.call_navigation == CallNavigation::UnresolvedSyntax,
                    index == 4
                );
            }
            // A malformed source body must not erase independently admitted
            // compiler-call navigation; a forged declaration grants neither.
            owner.normalized["documentation"]["events"] = json!("not an array");
            assert!(capabilities(&owner).unwrap().source_structure.is_none());
            owner.normalized["symbolIdentity"] = json!("forged");
            assert!(capabilities(&owner).is_none());
        }
    }

    #[test]
    fn roslyn_identity_prefix_and_collision_digest_match_the_worker_contract() {
        // A ref parameter can collide with a value parameter after portable
        // descriptor erasure. The worker salts the identity with the raw CLR ID.
        let raw = "M:Probe.Choose(System.Int32@)";
        let clr = format!("csharp:{raw}");
        let base = "method:class:Probe#Choose(I)I";
        let symbol = format!(
            "{base}@{}",
            &crate::canonical::hash_bytes(raw.as_bytes())[7..19]
        );
        let normalized = json!({"schema":crate::csharp_adapter_v2::CSHARP_FACT_SCHEMA,
            "kind":"DECLARATION","declarationKind":"METHOD","resolution":"COMPILER_EXACT",
            "symbolIdentity":symbol,"ownerIdentity":"class:Probe","name":"Choose",
            "jvmDescriptor":"(I)I","csharpIdentity":clr,"scope":"probe"});
        let mut owner = Observation {
            id: "choose".into(),
            kind: "SYMBOL".into(),
            service: "probe".into(),
            symbol,
            digest: digest(&normalized).unwrap(),
            normalized,
            source_ids: vec![],
        };
        assert!(csharp_admitted(&owner));
        owner.normalized["csharpIdentity"] = json!(raw);
        assert!(!csharp_admitted(&owner));
        owner.normalized["csharpIdentity"] = json!(clr);
        owner.symbol = format!(
            "{base}@{}",
            &crate::canonical::hash_bytes(clr.as_bytes())[7..19]
        );
        owner.normalized["symbolIdentity"] = json!(owner.symbol);
        assert!(!csharp_admitted(&owner));
    }
}
