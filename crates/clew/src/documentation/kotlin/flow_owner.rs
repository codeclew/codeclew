use super::{DeclarationOccurrence, declaration_occurrence, fact_scope, semantic_inputs};
use crate::error::{ClewError, ErrorCode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum Mode {
    Explicit,
    Legacy,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum KeyPart {
    Owner,
    Symbol,
    Scope,
    File,
    Range,
    Occurrence,
    Flow,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum Cardinality {
    Zero,
    One,
    Multiple,
}

impl Cardinality {
    fn of(matches: impl Iterator) -> Self {
        match matches.take(2).count() {
            0 => Self::Zero,
            1 => Self::One,
            _ => Self::Multiple,
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Diagnostic {
    mode: Mode,
    key_part: KeyPart,
    matches: Cardinality,
    // Candidate occurrences after matching the symbol, scope and file,
    // before requiring an explicit range.
    #[serde(skip_serializing_if = "Option::is_none")]
    same_file_candidates: Option<Cardinality>,
}

fn binding_error(
    mode: Mode,
    key_part: KeyPart,
    matches: Cardinality,
    same_file_candidates: Option<Cardinality>,
) -> ClewError {
    Diagnostic {
        mode,
        key_part,
        matches,
        same_file_candidates,
    }
    .error("Kotlin documentation flow lacks one exact scope/file/declaration owner")
}

impl Diagnostic {
    fn error(self, message: &'static str) -> ClewError {
        let mut error = ClewError::new(ErrorCode::InvalidInput, message);
        error.details = Some(Box::new(json!({"kotlinFlowOwner": self})));
        error
    }
}

pub(in crate::documentation) fn summary(value: &Value) -> Option<Value> {
    let diagnostic: Diagnostic = serde_json::from_value(value.clone()).ok()?;
    serde_json::to_value(diagnostic).ok()
}

#[derive(Clone, Copy)]
enum Descriptor {
    CompleteFunction,
    CoreFunction,
    Ineligible,
    Multiple,
}

type FileRanges = BTreeMap<(String, u64, u64), Descriptor>;
type OwnerIndex = BTreeMap<String, BTreeMap<String, FileRanges>>;
type Flows = BTreeMap<DeclarationOccurrence, (Value, String)>;
type AmbiguousSymbols = BTreeSet<(String, String)>;

fn index(facts: &[(Value, String)]) -> Result<OwnerIndex, ClewError> {
    let mut owners = OwnerIndex::new();
    for (fact, _) in facts {
        if fact["schema"] != "declaration-descriptor/0.1" {
            continue;
        }
        let Some((scope, file, symbol, start, end)) = declaration_occurrence(fact) else {
            continue;
        };
        let is_partial =
            fact.get("attributeCoverage").is_some() || fact.get("sourceRowHash").is_some();
        let descriptor = if fact["declarationKind"] != "FUNCTION" {
            if is_partial {
                continue;
            }
            Descriptor::Ineligible
        } else if is_partial {
            crate::semantic_validation::validate_declaration_descriptor_fact(
                &super::without_capture_scope(fact),
            )?;
            // Attribute coverage can be partial while the source occurrence
            // and the compiler/JVM identity are both usable. An opaque JVM
            // signature is still insufficient for a compiler-linked flow.
            if crate::semantic_validation::validate_kotlin_full_symbol_identity(&symbol).is_ok() {
                Descriptor::CoreFunction
            } else {
                Descriptor::Ineligible
            }
        } else {
            Descriptor::CompleteFunction
        };
        owners
            .entry(symbol)
            .or_default()
            .entry(scope)
            .or_default()
            .entry((file, start, end))
            .and_modify(|existing| *existing = Descriptor::Multiple)
            .or_insert(descriptor);
    }
    Ok(owners)
}

fn bind(
    owners: &OwnerIndex,
    fact: &Value,
    declaration: &Value,
    mode: Mode,
) -> Result<DeclarationOccurrence, ClewError> {
    let failure = |part, matches, candidates| binding_error(mode, part, matches, candidates);
    let owner = declaration.get("documentationOwner");
    if owner.is_some_and(|value| !value.is_object()) {
        return Err(failure(KeyPart::Owner, Cardinality::Zero, None));
    }
    let symbol = declaration["documentationSymbol"]
        .as_str()
        .filter(|symbol| !symbol.is_empty())
        .ok_or_else(|| failure(KeyPart::Symbol, Cardinality::Zero, None))?;
    if owner.is_some_and(|value| value["symbolIdentity"].as_str() != Some(symbol)) {
        return Err(failure(KeyPart::Symbol, Cardinality::Zero, None));
    }
    let scope = fact_scope(fact);
    if scope.is_empty() {
        return Err(failure(KeyPart::Scope, Cardinality::Zero, None));
    }
    let file = fact["path"]
        .as_str()
        .filter(|file| !file.is_empty())
        .ok_or_else(|| failure(KeyPart::File, Cardinality::Zero, None))?;
    if declaration
        .get("file")
        .is_some_and(|value| value.as_str() != Some(file))
        || owner.is_some_and(|value| value["file"].as_str() != Some(file))
    {
        return Err(failure(KeyPart::File, Cardinality::Zero, None));
    }

    let scopes = owners
        .get(symbol)
        .ok_or_else(|| failure(KeyPart::Symbol, Cardinality::Zero, None))?;
    let ranges = scopes
        .get(&scope)
        .ok_or_else(|| failure(KeyPart::Scope, Cardinality::Zero, None))?;
    let same_file = ranges.range((file.into(), 0, 0)..=(file.into(), u64::MAX, u64::MAX));
    let count = match same_file.clone().next() {
        Some((_, Descriptor::Multiple)) => Cardinality::Multiple,
        _ => Cardinality::of(same_file.clone()),
    };
    if matches!(count, Cardinality::Zero) {
        return Err(failure(KeyPart::File, Cardinality::Zero, None));
    }

    let ((_, start, end), descriptor) = if let Some(owner) = owner {
        let (start, end) = owner["byteStart"]
            .as_u64()
            .zip(owner["byteEnd"].as_u64())
            .filter(|(start, end)| start < end)
            .ok_or_else(|| failure(KeyPart::Range, Cardinality::Zero, Some(count)))?;
        ranges
            .get_key_value(&(file.into(), start, end))
            .ok_or_else(|| failure(KeyPart::Range, Cardinality::Zero, Some(count)))?
    } else {
        // Older file rows omit the range. Keep support for one complete
        // descriptor in the same scope and file, with no wider fallback.
        if matches!(count, Cardinality::Multiple) {
            return Err(failure(KeyPart::Occurrence, Cardinality::Multiple, None));
        }
        same_file
            .into_iter()
            .next()
            .ok_or_else(|| failure(KeyPart::Occurrence, Cardinality::Zero, None))?
    };
    match (descriptor, mode) {
        (Descriptor::CompleteFunction, _) | (Descriptor::CoreFunction, Mode::Explicit) => {}
        (Descriptor::Multiple, _) => {
            return Err(failure(KeyPart::Occurrence, Cardinality::Multiple, None));
        }
        _ => return Err(failure(KeyPart::Occurrence, Cardinality::One, None)),
    }
    Ok((scope, file.into(), symbol.into(), *start, *end))
}

pub(super) fn collect(facts: &[(Value, String)]) -> Result<(Flows, AmbiguousSymbols), ClewError> {
    let owners = index(facts)?;
    let mut flows = Flows::new();
    for (fact, binding) in facts {
        for declaration in fact["declarations"].as_array().into_iter().flatten() {
            let Some(flow) = declaration.get("documentation") else {
                continue;
            };
            let mode = if declaration.get("documentationOwner").is_some() {
                Mode::Explicit
            } else {
                Mode::Legacy
            };
            let key = bind(&owners, fact, declaration, mode)?;
            if !flow.is_object() {
                return Err(binding_error(mode, KeyPart::Flow, Cardinality::One, None));
            }
            if let Some(metadata) = flow.get("semanticInputs") {
                // The semantic carrier is produced independently of the file
                // row's owner reference. It must agree even when a bad range
                // happens to name another real declaration in the same file.
                for (part, agrees) in [
                    (KeyPart::Scope, metadata["compilationScope"] == key.0),
                    (KeyPart::File, metadata["file"] == key.1),
                    (KeyPart::Symbol, metadata["ownerSymbolIdentity"] == key.2),
                    (
                        KeyPart::Range,
                        metadata["ownerByteStart"] == key.3 && metadata["ownerByteEnd"] == key.4,
                    ),
                ] {
                    if !agrees {
                        return Err(binding_error(mode, part, Cardinality::One, None));
                    }
                }
                semantic_inputs::validate(metadata, &key, fact["contentHash"].as_str())?;
                semantic_inputs::validate_event_carriers(flow)?;
            }
            if flows.insert(key, (flow.clone(), binding.clone())).is_some() {
                return Err(Diagnostic {
                    mode,
                    key_part: KeyPart::Flow,
                    matches: Cardinality::Multiple,
                    same_file_candidates: None,
                }
                .error("Kotlin documentation repeats a compiler declaration flow owner"));
            }
        }
    }
    let mut ambiguous = AmbiguousSymbols::new();
    for (symbol, scopes) in owners {
        for (scope, ranges) in scopes {
            if ranges
                .values()
                .any(|descriptor| matches!(descriptor, Descriptor::Multiple))
            {
                return Err(super::invalid(
                    "Kotlin documentation repeats an exact compiler declaration occurrence",
                ));
            }
            if ranges.len() > 1 {
                ambiguous.insert((scope, symbol.clone()));
            }
        }
    }
    Ok((flows, ambiguous))
}
