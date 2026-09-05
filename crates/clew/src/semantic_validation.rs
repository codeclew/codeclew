//! Closed validation contour for compiler-produced Kotlin semantic graphs.
//!
//! This module deliberately contains no repository persistence, freshness, or
//! identity-reconciliation behavior. It validates one sealed worker response
//! and returns only the hashes and provenance needed to issue a live capability.

use crate::canonical;
use crate::error::{ClewError, ErrorCode};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeclarationRelationSnapshot {
    pub(crate) graph: Value,
    pub(crate) hash: String,
    pub(crate) provenance: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeclarationDescriptorSnapshot {
    pub(crate) graph: Value,
    pub(crate) hash: String,
    pub(crate) provenance: Value,
}

fn declaration_source_binding(facts: &Value) -> Result<Value, ClewError> {
    let invalid = |message: &str| ClewError::new(ErrorCode::InvalidInput, message);
    let compilation = facts
        .get("compilation")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| invalid("semantic source binding has no compilation"))?;
    let files = facts
        .get("files")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("semantic source binding has no source files"))?;
    let mut rows = Vec::with_capacity(files.len());
    let mut paths = BTreeSet::new();
    for file in files {
        let path = file
            .get("path")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| invalid("semantic source binding file has no path"))?;
        let module = file
            .get("module")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| invalid("semantic source binding file has no module"))?;
        let source_set = file
            .get("sourceSet")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| invalid("semantic source binding file has no sourceSet"))?;
        let path_value = Path::new(path);
        if path_value.is_absolute()
            || path_value.components().any(|part| {
                !matches!(
                    part,
                    std::path::Component::Normal(_) | std::path::Component::CurDir
                )
            })
            || !paths.insert(path)
            || format!("{module}/{source_set}") != compilation
        {
            return Err(ClewError::new(
                ErrorCode::ProjectModelChanged,
                "semantic source binding path/module/sourceSet differs from compilation",
            ));
        }
        rows.push(serde_json::json!({
            "path":path,
            "module":module,
            "sourceSet":source_set,
        }));
    }
    rows.sort_by_key(|value| canonical::bytes(value).unwrap_or_default());
    Ok(serde_json::json!({
        "schema":"declaration-source-binding/0.1",
        "compilation":compilation,
        "files":rows,
    }))
}

fn is_canonical_sha256(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|digest| {
        digest.len() == 64
            && digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)]
pub(crate) enum KotlinSemanticPayloadKind {
    DeclarationDescriptor,
    DeclarationRelation,
    DeclarationDescriptorBoundary,
    DeclarationRelationBoundary,
}

fn payload_invalid(message: impl Into<String>) -> ClewError {
    ClewError::new(ErrorCode::InvalidInput, message)
}

fn payload_string<'a>(value: &'a Value, field: &str) -> Result<&'a str, ClewError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| payload_invalid(format!("semantic payload has no nonempty {field}")))
}

fn closed_payload(value: &Value, allowed: &[&str], label: &str) -> Result<(), ClewError> {
    let object = value
        .as_object()
        .ok_or_else(|| payload_invalid(format!("{label} is not an object")))?;
    if object
        .keys()
        .any(|field| !allowed.contains(&field.as_str()))
    {
        return Err(payload_invalid(format!(
            "{label} has a field outside its exact contract"
        )));
    }
    Ok(())
}

fn validate_payload_location(value: &Value, label: &str) -> Result<(), ClewError> {
    let file = payload_string(value, "file")?;
    let path = Path::new(file);
    if path.is_absolute()
        || path.components().any(|part| {
            !matches!(
                part,
                std::path::Component::Normal(_) | std::path::Component::CurDir
            )
        })
    {
        return Err(payload_invalid(format!(
            "{label} source path is not repository-contained"
        )));
    }
    let start = value
        .get("start")
        .and_then(Value::as_i64)
        .ok_or_else(|| payload_invalid(format!("{label} has no source start")))?;
    let end = value
        .get("end")
        .and_then(Value::as_i64)
        .ok_or_else(|| payload_invalid(format!("{label} has no source end")))?;
    if start < 0 || end < start {
        return Err(payload_invalid(format!(
            "{label} has an invalid source range"
        )));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct ParsedJvmMethodDescriptor {
    returns_void: bool,
    parameter_count: usize,
}

struct JvmMethodDescriptorParser<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl JvmMethodDescriptorParser<'_> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.offset).copied()
    }

    fn field_type(&mut self) -> Result<u16, ClewError> {
        let mut dimensions = 0_u16;
        while self.peek() == Some(b'[') {
            dimensions += 1;
            if dimensions > 255 {
                return Err(payload_invalid(
                    "JVM method descriptor array has more than 255 dimensions",
                ));
            }
            self.offset += 1;
        }
        match self.peek() {
            Some(token @ (b'B' | b'C' | b'D' | b'F' | b'I' | b'J' | b'S' | b'Z')) => {
                self.offset += 1;
                Ok(if dimensions == 0 && matches!(token, b'D' | b'J') {
                    2
                } else {
                    1
                })
            }
            Some(b'L') => self.object_type().map(|()| 1),
            Some(b'V') if dimensions > 0 => Err(payload_invalid(
                "JVM method descriptor array component cannot be void",
            )),
            Some(b'V') => Err(payload_invalid(
                "JVM method descriptor parameter cannot be void",
            )),
            Some(_) => Err(payload_invalid(
                "JVM method descriptor contains an unknown field type",
            )),
            None => Err(payload_invalid(
                "JVM method descriptor has an incomplete field type",
            )),
        }
    }

    fn object_type(&mut self) -> Result<(), ClewError> {
        self.offset += 1;
        let mut segment_length = 0_usize;
        loop {
            let byte = self.peek().ok_or_else(|| {
                payload_invalid("JVM method descriptor has an unterminated object type")
            })?;
            match byte {
                b';' if segment_length > 0 => {
                    self.offset += 1;
                    return Ok(());
                }
                b';' | b'/' if segment_length == 0 => {
                    return Err(payload_invalid(
                        "JVM method descriptor object type has an empty name segment",
                    ));
                }
                b'/' => {
                    self.offset += 1;
                    segment_length = 0;
                }
                b'.' | b'[' | b'(' | b')' | 0..=31 | 127 => {
                    return Err(payload_invalid(
                        "JVM method descriptor object type has an invalid class name",
                    ));
                }
                _ => {
                    self.offset += 1;
                    segment_length += 1;
                }
            }
        }
    }
}

fn parse_jvm_method_descriptor(descriptor: &str) -> Result<ParsedJvmMethodDescriptor, ClewError> {
    let mut parser = JvmMethodDescriptorParser {
        bytes: descriptor.as_bytes(),
        offset: 0,
    };
    if parser.peek() != Some(b'(') {
        return Err(payload_invalid(
            "JVM method descriptor has no opening parameter delimiter",
        ));
    }
    parser.offset += 1;
    let mut parameter_count = 0_usize;
    let mut parameter_slots = 0_u16;
    loop {
        match parser.peek() {
            Some(b')') => {
                parser.offset += 1;
                break;
            }
            Some(_) => {
                let slots = parser.field_type()?;
                parameter_slots = parameter_slots.checked_add(slots).ok_or_else(|| {
                    payload_invalid("JVM method descriptor parameter length overflow")
                })?;
                if parameter_slots > 255 {
                    return Err(payload_invalid(
                        "JVM method descriptor parameter length exceeds 255 units",
                    ));
                }
                parameter_count = parameter_count.checked_add(1).ok_or_else(|| {
                    payload_invalid("JVM method descriptor parameter count overflow")
                })?;
            }
            None => {
                return Err(payload_invalid(
                    "JVM method descriptor has no closing parameter delimiter",
                ));
            }
        }
    }
    let returns_void = if parser.peek() == Some(b'V') {
        parser.offset += 1;
        true
    } else {
        parser.field_type()?;
        false
    };
    if parser.offset != parser.bytes.len() {
        return Err(payload_invalid("JVM method descriptor has trailing bytes"));
    }
    Ok(ParsedJvmMethodDescriptor {
        returns_void,
        parameter_count,
    })
}

pub(crate) fn validate_jvm_method_descriptor(descriptor: &str) -> Result<(), ClewError> {
    parse_jvm_method_descriptor(descriptor).map(|_| ())
}

fn validate_jvm_function_signature(signature: &str) -> Result<(), ClewError> {
    if signature.starts_with('(') {
        return validate_jvm_method_descriptor(signature);
    }
    let delimiter = signature
        .find('(')
        .ok_or_else(|| payload_invalid("JVM function signature has no method descriptor"))?;
    let (name, descriptor) = signature.split_at(delimiter);
    if name.is_empty()
        || name
            .bytes()
            .any(|byte| matches!(byte, b'.' | b';' | b'[' | b'/' | b'<' | b'>' | 0..=31 | 127))
    {
        return Err(payload_invalid(
            "JVM function signature has an invalid method name",
        ));
    }
    validate_jvm_method_descriptor(descriptor)
}

/// A retained PARTIAL descriptor does not claim an exact JVM signature. The
/// compiler normalizer keeps the opaque suffix only to preserve a stable link
/// to the original callable while a paired UNKNOWN boundary records why the
/// attributed row could not be proven. Keep that identity bounded and inert;
/// exact descriptors still pass through the strict JVM grammar above.
fn validate_partial_jvm_signature(signature: &str) -> Result<(), ClewError> {
    if signature.is_empty()
        || signature.chars().any(char::is_control)
        || signature.contains("#jvm:")
        || signature.contains("://")
        || signature
            .chars()
            .any(|character| matches!(character, '\\' | '@'))
        || !signature.contains('(')
        || !signature.contains(')')
    {
        return Err(payload_invalid(
            "partial JVM signature is not a safe opaque compiler identity",
        ));
    }
    Ok(())
}

fn validate_raw_compiler_identity(
    identity: &str,
    label: &str,
    allow_root_package_callable: bool,
) -> Result<(), ClewError> {
    let identity = match identity.strip_prefix('/') {
        Some(root) if allow_root_package_callable && !root.contains('/') => root,
        _ => identity,
    };
    if identity.is_empty()
        || identity.chars().any(char::is_control)
        || identity.starts_with('/')
        || identity
            .chars()
            .any(|character| matches!(character, ':' | '\\' | '@'))
        || identity
            .split('/')
            .any(|segment| segment.is_empty() || matches!(segment, "." | ".."))
        || identity.contains("#jvm:")
        || identity.contains("<unknown>")
        || identity.contains("<unresolved>")
        || identity == "UNKNOWN"
        || [
            "callable:",
            "constructor:",
            "property:",
            "class:",
            "package:",
        ]
        .iter()
        .any(|prefix| identity.starts_with(prefix))
    {
        return Err(payload_invalid(format!(
            "{label} is not a raw K2 compiler identity"
        )));
    }
    Ok(())
}

fn validate_relation_endpoint(identity: &str, label: &str) -> Result<(), ClewError> {
    if let Some(tagged) = identity.strip_prefix("callable:") {
        let (callable, signature) = tagged.split_once("#jvm:").ok_or_else(|| {
            payload_invalid(format!("{label} callable symbol has no JVM signature"))
        })?;
        validate_raw_compiler_identity(callable, label, true)?;
        return validate_jvm_function_signature(signature);
    }
    if let Some(tagged) = identity.strip_prefix("constructor:") {
        let (callable, descriptor) = tagged.split_once("#jvm:").ok_or_else(|| {
            payload_invalid(format!("{label} constructor symbol has no JVM descriptor"))
        })?;
        validate_raw_compiler_identity(callable, label, false)?;
        if !parse_jvm_method_descriptor(descriptor)?.returns_void {
            return Err(payload_invalid(format!(
                "{label} constructor descriptor does not return void"
            )));
        }
        return Ok(());
    }
    if let Some(raw) = identity.strip_prefix("property:") {
        return validate_raw_compiler_identity(raw, label, true);
    }
    for prefix in ["class:", "package:"] {
        if let Some(raw) = identity.strip_prefix(prefix) {
            return validate_raw_compiler_identity(raw, label, false);
        }
    }
    // K2 renders a callable or class in the root package as `/name` in raw
    // relation endpoints. A single segment is compiler identity syntax; a
    // second slash still makes the value fail closed as a path lookalike.
    validate_raw_compiler_identity(identity, label, true)
}

pub(crate) fn validate_kotlin_full_symbol_identity(identity: &str) -> Result<(), ClewError> {
    if identity.contains("://") || identity.contains('\\') || identity.contains('@') {
        return Err(payload_invalid(
            "Kotlin full symbol identity contains URL or credential syntax",
        ));
    }
    if !["callable:", "constructor:", "property:", "class:"]
        .iter()
        .any(|prefix| identity.starts_with(prefix))
    {
        return Err(payload_invalid(
            "Kotlin full symbol identity has no supported declaration tag",
        ));
    }
    validate_relation_endpoint(identity, "Kotlin full symbol identity")
}

fn descriptor_allowed_fields(kind: &str, partial: bool) -> Result<Vec<&'static str>, ClewError> {
    let mut allowed = vec![
        "schema",
        "file",
        "start",
        "end",
        "startLine",
        "endLine",
        "lineProvenance",
        "symbolIdentity",
        "declarationKind",
        "ownerIdentity",
        "containment",
        "resolution",
        "provider",
        "module",
        "sourceSet",
        "sourceProvenance",
        "compilerAuthority",
    ];
    if partial {
        allowed.extend(["attributeCoverage", "sourceRowHash"]);
    } else {
        allowed.extend([
            "visibility",
            "effectiveVisibility",
            "exportBoundary",
            "modality",
            "typeParameters",
        ]);
    }
    match kind {
        "FUNCTION" => {
            allowed.extend(["compilerCallableId", "jvmDescriptor"]);
            if !partial {
                allowed.extend([
                    "isOverride",
                    "spring",
                    "returnType",
                    "returnNullable",
                    "parameterTypes",
                    "receiverType",
                ]);
            }
        }
        "CONSTRUCTOR" => {
            allowed.extend(["compilerCallableId", "compilerClassId", "jvmDescriptor"]);
            if !partial {
                allowed.extend(["isPrimary", "parameterTypes"]);
            }
        }
        "PROPERTY" | "MUTABLE_PROPERTY" => {
            allowed.push("compilerCallableId");
            if !partial {
                allowed.extend(["isOverride", "declaredType", "declaredNullable"]);
            }
        }
        "CLASS" => allowed.extend(["compilerClassId", "spring"]),
        _ => return Err(payload_invalid("unknown declaration descriptor kind")),
    }
    Ok(allowed)
}

fn validate_optional_descriptor_lines(value: &Value) -> Result<(), ClewError> {
    let present = ["startLine", "endLine", "lineProvenance"]
        .iter()
        .filter(|field| value.get(**field).is_some())
        .count();
    if present == 0 {
        return Ok(());
    }
    let start = value.get("startLine").and_then(Value::as_u64);
    let end = value.get("endLine").and_then(Value::as_u64);
    if present != 3
        || start.is_none_or(|line| line == 0)
        || end.is_none_or(|line| line == 0)
        || start.zip(end).is_none_or(|(start, end)| start > end)
        || value.get("lineProvenance").and_then(Value::as_str)
            != Some("UTF8_BYTE_RANGE_OVER_COMPILATION_SOURCE")
    {
        return Err(payload_invalid(
            "declaration descriptor has invalid line coordinates",
        ));
    }
    Ok(())
}

pub(crate) fn validate_declaration_descriptor_fact(value: &Value) -> Result<(), ClewError> {
    if value.get("schema").and_then(Value::as_str) != Some("declaration-descriptor/0.1")
        || value.get("resolution").and_then(Value::as_str) != Some("PROVEN")
        || value.get("provider").and_then(Value::as_str) != Some("K2_FIR")
        || value.get("sourceProvenance").and_then(Value::as_str)
            != Some("COMPILER_UTF16_RANGE_TO_UTF8_BYTES")
        || value.get("compilerAuthority").and_then(Value::as_str) != Some("fir-facts-extractor/0.6")
    {
        return Err(payload_invalid(
            "malformed or non-authoritative declaration descriptor",
        ));
    }
    validate_payload_location(value, "declaration descriptor")?;
    validate_optional_descriptor_lines(value)?;
    if let Some(spring) = value.get("spring") {
        crate::spring_entrypoints::validate_metadata(spring, "K2_RESOLVED_ANNOTATIONS")?;
    }
    for field in ["symbolIdentity", "ownerIdentity", "module", "sourceSet"] {
        payload_string(value, field)?;
    }
    if !value.get("containment").is_some_and(Value::is_array) {
        return Err(payload_invalid(
            "declaration descriptor has no containment array",
        ));
    }
    let kind = payload_string(value, "declarationKind")?;
    let partial = value.get("attributeCoverage").is_some() || value.get("sourceRowHash").is_some();
    closed_payload(
        value,
        &descriptor_allowed_fields(kind, partial)?,
        "declaration descriptor",
    )?;
    if partial {
        let source_row_hash = payload_string(value, "sourceRowHash")?;
        if value.get("attributeCoverage").and_then(Value::as_str) != Some("PARTIAL")
            || !is_canonical_sha256(source_row_hash)
        {
            return Err(payload_invalid(
                "partial declaration descriptor has invalid core authority",
            ));
        }
    }
    match kind {
        "FUNCTION" => {
            let callable = payload_string(value, "compilerCallableId")?;
            validate_raw_compiler_identity(callable, "function compilerCallableId", true)?;
            let prefix = format!("callable:{callable}#jvm:");
            let signature = payload_string(value, "symbolIdentity")?
                .strip_prefix(&prefix)
                .ok_or_else(|| payload_invalid("function symbol identity is inconsistent"))?;
            if partial {
                validate_partial_jvm_signature(signature)?;
            } else {
                validate_jvm_function_signature(signature)?;
                if let Some(descriptor) = value.get("jvmDescriptor") {
                    let descriptor = descriptor.as_str().ok_or_else(|| {
                        payload_invalid("function JVM descriptor is not a string")
                    })?;
                    validate_jvm_method_descriptor(descriptor)?;
                    if signature != descriptor {
                        return Err(payload_invalid(
                            "function symbol identity disagrees with JVM descriptor",
                        ));
                    }
                }
            }
        }
        "CONSTRUCTOR" => {
            let callable = payload_string(value, "compilerCallableId")?;
            let class = payload_string(value, "compilerClassId")?;
            let descriptor = payload_string(value, "jvmDescriptor")?;
            validate_raw_compiler_identity(callable, "constructor compilerCallableId", false)?;
            validate_raw_compiler_identity(class, "constructor compilerClassId", false)?;
            if payload_string(value, "symbolIdentity")?
                != format!("constructor:{callable}#jvm:{descriptor}")
                || payload_string(value, "ownerIdentity")? != format!("class:{class}")
            {
                return Err(payload_invalid(
                    "constructor compiler/JVM identity is inconsistent",
                ));
            }
            if partial {
                validate_partial_jvm_signature(descriptor)?;
            } else if !parse_jvm_method_descriptor(descriptor)?.returns_void {
                return Err(payload_invalid(
                    "constructor compiler/JVM identity is inconsistent",
                ));
            }
        }
        "PROPERTY" | "MUTABLE_PROPERTY" => {
            let callable = payload_string(value, "compilerCallableId")?;
            validate_raw_compiler_identity(callable, "property compilerCallableId", true)?;
            if payload_string(value, "symbolIdentity")? != format!("property:{callable}") {
                return Err(payload_invalid(
                    "property compiler identity is inconsistent",
                ));
            }
        }
        "CLASS" => {
            let class = payload_string(value, "compilerClassId")?;
            validate_raw_compiler_identity(class, "class compilerClassId", false)?;
            if payload_string(value, "symbolIdentity")? != format!("class:{class}") {
                return Err(payload_invalid("class compiler identity is inconsistent"));
            }
        }
        _ => unreachable!(),
    }
    Ok(())
}

/// Return true only when an otherwise valid exact descriptor fails solely
/// because its compiler-emitted JVM suffix is safe to retain as opaque UNKNOWN
/// evidence but is not valid JVM descriptor grammar. Admission may quarantine
/// such a row; it must never persist it as a PROVEN descriptor.
pub(crate) fn has_quarantinable_exact_jvm_descriptor(value: &Value) -> bool {
    if value.get("attributeCoverage").is_some()
        || value.get("sourceRowHash").is_some()
        || validate_declaration_descriptor_fact(value).is_ok()
    {
        return false;
    }
    let kind = value.get("declarationKind").and_then(Value::as_str);
    let mut normalized = value.clone();
    match kind {
        Some("FUNCTION") => {
            let Some(callable) = value.get("compilerCallableId").and_then(Value::as_str) else {
                return false;
            };
            let prefix = format!("callable:{callable}#jvm:");
            let Some(signature) = value
                .get("symbolIdentity")
                .and_then(Value::as_str)
                .and_then(|identity| identity.strip_prefix(&prefix))
            else {
                return false;
            };
            if validate_partial_jvm_signature(signature).is_err() {
                return false;
            }
            if let Some(descriptor) = value.get("jvmDescriptor") {
                let Some(descriptor) = descriptor.as_str() else {
                    return false;
                };
                if descriptor != signature {
                    return false;
                }
                normalized["jvmDescriptor"] = Value::String("()V".to_owned());
            }
            normalized["symbolIdentity"] = Value::String(format!("{prefix}()V"));
        }
        Some("CONSTRUCTOR") => {
            let Some(callable) = value.get("compilerCallableId").and_then(Value::as_str) else {
                return false;
            };
            let Some(descriptor) = value.get("jvmDescriptor").and_then(Value::as_str) else {
                return false;
            };
            if validate_partial_jvm_signature(descriptor).is_err()
                || value.get("symbolIdentity").and_then(Value::as_str)
                    != Some(format!("constructor:{callable}#jvm:{descriptor}").as_str())
            {
                return false;
            }
            normalized["jvmDescriptor"] = Value::String("()V".to_owned());
            normalized["symbolIdentity"] = Value::String(format!("constructor:{callable}#jvm:()V"));
        }
        _ => return false,
    }
    validate_declaration_descriptor_fact(&normalized).is_ok()
}

fn relation_allowed_fields(kind: &str, partial: bool) -> Result<Vec<&'static str>, ClewError> {
    let mut allowed = vec![
        "schema",
        "file",
        "start",
        "end",
        "kind",
        "owner",
        "target",
        "resolution",
        "provider",
        "cfgNodeIds",
        "sourceProvenance",
        "orderProvenance",
    ];
    if partial {
        allowed.extend(["attributeCoverage", "sourceRowHash"]);
        if matches!(kind, "CALLS" | "CONSTRUCTS") {
            allowed.extend(["targetCompilerCallableId", "targetJvmDescriptor"]);
        }
        return Ok(allowed);
    }
    match kind {
        "OVERRIDES" => allowed.extend([
            "sourceReturnType",
            "baseReturnType",
            "sourceParameterTypes",
            "baseParameterTypes",
        ]),
        "CALLS" | "CONSTRUCTS" => allowed.extend([
            "targetCompilerCallableId",
            "targetJvmDescriptor",
            "receiverSelection",
            "omittedDefaultParameterIndices",
            "resultType",
            "receiverType",
            "argumentToParameter",
            "orderKey",
        ]),
        "REFERENCES" => allowed.extend(["resultType", "receiverType"]),
        // K2 emits an empty argument map and order key for a resolved property read.
        "READS" => allowed.extend([
            "resultType",
            "receiverType",
            "argumentToParameter",
            "orderKey",
        ]),
        "WRITES" | "INITIALIZES" => allowed.extend(["valueType", "targetType", "orderKey"]),
        "NULL_COALESCES" => allowed.extend([
            "sourceTarget",
            "fallbackTarget",
            "sourceOccurrence",
            "fallbackOccurrence",
            "mergedOccurrence",
            "branchProvenance",
            "orderKey",
        ]),
        "RETURNS_VALUE_FROM" => allowed.extend([
            "sourceKind",
            "sourceOccurrence",
            "returnOccurrence",
            "resultOccurrence",
            "resultType",
            "resultNullable",
            "valueProvenance",
            "cfgProvenance",
            "evaluationCount",
            "orderKey",
        ]),
        _ => return Err(payload_invalid("unknown declaration relation kind")),
    }
    Ok(allowed)
}

fn validate_exact_call_target(
    value: &Value,
    kind: &str,
    partial: bool,
) -> Result<Option<usize>, ClewError> {
    let target = payload_string(value, "target")?;
    let callable_field = value.get("targetCompilerCallableId");
    let descriptor_field = value.get("targetJvmDescriptor");
    if callable_field.is_none() && descriptor_field.is_none() {
        return Ok(None);
    }
    if callable_field.is_none() || descriptor_field.is_none() {
        return Err(payload_invalid(
            "call target has only one exact compiler identity field",
        ));
    }
    let callable = payload_string(value, "targetCompilerCallableId")?;
    let descriptor = payload_string(value, "targetJvmDescriptor")?;
    validate_raw_compiler_identity(callable, "call target compiler callable id", false)?;
    let parsed = parse_jvm_method_descriptor(descriptor)?;
    let expected = match kind {
        "CALLS" => format!("callable:{callable}#jvm:{descriptor}"),
        "CONSTRUCTS" => {
            if !parsed.returns_void {
                return Err(payload_invalid(
                    "constructor call target descriptor does not return void",
                ));
            }
            format!("constructor:{callable}#jvm:{descriptor}")
        }
        _ => {
            return Err(payload_invalid(
                "non-call relation has an exact call target",
            ));
        }
    };
    if target != expected {
        return Err(payload_invalid(
            "call target identity disagrees with compiler callable id or JVM descriptor",
        ));
    }
    if !partial {
        let receiver_selection = payload_string(value, "receiverSelection")?;
        match kind {
            "CALLS"
                if receiver_selection != "EXPLICIT"
                    || payload_string(value, "receiverType").is_err() =>
            {
                return Err(payload_invalid(
                    "exact call target has no explicit receiver authority",
                ));
            }
            "CONSTRUCTS" if receiver_selection != "NONE" || value.get("receiverType").is_some() => {
                return Err(payload_invalid(
                    "exact constructor target has invalid receiver authority",
                ));
            }
            _ => {}
        }
        if !value
            .get("omittedDefaultParameterIndices")
            .is_some_and(Value::is_array)
        {
            return Err(payload_invalid(
                "exact call target has no omitted-default parameter set",
            ));
        }
    }
    Ok(Some(parsed.parameter_count))
}

fn validate_argument_payloads(
    value: &Value,
    target_parameter_count: Option<usize>,
) -> Result<(), ClewError> {
    let Some(arguments) = value.get("argumentToParameter") else {
        return Ok(());
    };
    let arguments = arguments
        .as_array()
        .ok_or_else(|| payload_invalid("declaration relation argument mapping is not an array"))?;
    let relation_start = value.get("start").and_then(Value::as_i64).unwrap_or(-1);
    let relation_end = value.get("end").and_then(Value::as_i64).unwrap_or(-1);
    let mut argument_ranges = BTreeSet::new();
    let mut parameter_indices = BTreeSet::new();
    let mut previous_start = None;
    for argument in arguments {
        closed_payload(
            argument,
            &[
                "argumentStart",
                "argumentEnd",
                "argumentName",
                "argumentType",
                "parameter",
                "parameterIndex",
                "parameterType",
            ],
            "declaration relation argument mapping",
        )?;
        let argument_start = argument
            .get("argumentStart")
            .and_then(Value::as_i64)
            .ok_or_else(|| payload_invalid("argument mapping has no source start"))?;
        let argument_end = argument.get("argumentEnd").and_then(Value::as_i64);
        let parameter_index = argument
            .get("parameterIndex")
            .and_then(Value::as_u64)
            .ok_or_else(|| payload_invalid("argument mapping has no parameter index"))?;
        if argument_start < relation_start
            || argument_start >= relation_end
            || argument_end.is_some_and(|end| end <= argument_start || end > relation_end)
            || target_parameter_count.is_some() && argument_end.is_none()
            || previous_start.is_some_and(|previous| previous >= argument_start)
            || !argument_ranges.insert((argument_start, argument_end))
            || !parameter_indices.insert(parameter_index)
            || target_parameter_count.is_some_and(|count| parameter_index as usize >= count)
            || payload_string(argument, "parameterType").is_err()
        {
            return Err(payload_invalid(
                "declaration relation argument mapping has an invalid endpoint payload",
            ));
        }
        previous_start = Some(argument_start);
        for field in ["argumentName", "argumentType", "parameter"] {
            if argument
                .get(field)
                .is_some_and(|field| field.as_str().is_none_or(str::is_empty))
            {
                return Err(payload_invalid(
                    "declaration relation argument mapping has a malformed optional field",
                ));
            }
        }
        if target_parameter_count.is_some() {
            payload_string(argument, "argumentType")?;
            payload_string(argument, "parameter")?;
        }
    }
    let omitted = value
        .get("omittedDefaultParameterIndices")
        .map(|indices| {
            indices
                .as_array()
                .ok_or_else(|| payload_invalid("omitted-default parameter set is not an array"))
        })
        .transpose()?;
    if target_parameter_count.is_some() && omitted.is_none() {
        return Err(payload_invalid(
            "exact call target has no omitted-default parameter set",
        ));
    }
    let mut omitted_indices = BTreeSet::new();
    let mut previous_omitted = None;
    for index in omitted.into_iter().flatten() {
        let index = index
            .as_u64()
            .ok_or_else(|| payload_invalid("omitted-default parameter index is invalid"))?;
        if previous_omitted.is_some_and(|previous| previous >= index)
            || !omitted_indices.insert(index)
            || parameter_indices.contains(&index)
            || target_parameter_count.is_some_and(|count| index as usize >= count)
        {
            return Err(payload_invalid(
                "omitted-default parameter indices are not canonical and disjoint",
            ));
        }
        previous_omitted = Some(index);
    }
    if target_parameter_count.is_some_and(|count| {
        parameter_indices.len() + omitted_indices.len() != count
            || (0..count as u64).any(|index| {
                !parameter_indices.contains(&index) && !omitted_indices.contains(&index)
            })
    }) {
        return Err(payload_invalid(
            "mapped and omitted-default parameters do not partition the target descriptor",
        ));
    }
    Ok(())
}

pub(crate) fn validate_declaration_relation_fact(value: &Value) -> Result<(), ClewError> {
    if value.get("schema").and_then(Value::as_str) != Some("declaration-relation/0.1")
        || value.get("resolution").and_then(Value::as_str) != Some("PROVEN")
        || value.get("provider").and_then(Value::as_str) != Some("K2_FIR")
        || value.get("sourceProvenance").and_then(Value::as_str)
            != Some("COMPILER_UTF16_RANGE_TO_UTF8_BYTES")
        || !matches!(
            value.get("orderProvenance").and_then(Value::as_str),
            Some("K2_FIR_CFG" | "FIR_SOURCE_RANGE" | "UNKNOWN")
        )
    {
        return Err(payload_invalid(
            "malformed or non-authoritative declaration relation",
        ));
    }
    validate_payload_location(value, "declaration relation")?;
    let owner = payload_string(value, "owner")?;
    let target = payload_string(value, "target")?;
    validate_relation_endpoint(owner, "declaration relation owner")?;
    validate_relation_endpoint(target, "declaration relation target")?;
    if !value.get("cfgNodeIds").is_some_and(Value::is_array) {
        return Err(payload_invalid(
            "declaration relation has no CFG node array",
        ));
    }
    let kind = payload_string(value, "kind")?;
    let partial = value.get("attributeCoverage").is_some() || value.get("sourceRowHash").is_some();
    closed_payload(
        value,
        &relation_allowed_fields(kind, partial)?,
        "declaration relation",
    )?;
    if partial {
        let source_row_hash = payload_string(value, "sourceRowHash")?;
        if !matches!(kind, "CALLS" | "CONSTRUCTS")
            || value.get("attributeCoverage").and_then(Value::as_str) != Some("PARTIAL")
            || !is_canonical_sha256(source_row_hash)
        {
            return Err(payload_invalid(
                "partial declaration relation is outside the retained topology contract",
            ));
        }
    }
    if let Some(receiver_selection) = value.get("receiverSelection") {
        let receiver_selection = receiver_selection
            .as_str()
            .ok_or_else(|| payload_invalid("call receiver selection is not a string"))?;
        if !matches!(
            receiver_selection,
            "EXPLICIT" | "EXTENSION" | "DISPATCH" | "NONE"
        ) || receiver_selection == "NONE" && value.get("receiverType").is_some()
            || receiver_selection != "NONE" && value.get("receiverType").is_none()
            || kind == "CONSTRUCTS" && receiver_selection != "NONE"
        {
            return Err(payload_invalid("call receiver authority is invalid"));
        }
    }
    let target_parameter_count = if matches!(kind, "CALLS" | "CONSTRUCTS") {
        validate_exact_call_target(value, kind, partial)?
    } else {
        None
    };
    if kind == "NULL_COALESCES" {
        for field in ["sourceTarget", "fallbackTarget"] {
            validate_relation_endpoint(payload_string(value, field)?, field)?;
        }
    }
    validate_argument_payloads(value, target_parameter_count)
}

fn validate_optional_boundary_location(value: &Value, label: &str) -> Result<(), ClewError> {
    if value.get("file").is_some() {
        let file = payload_string(value, "file")?;
        let path = Path::new(file);
        if path.is_absolute()
            || path.components().any(|part| {
                !matches!(
                    part,
                    std::path::Component::Normal(_) | std::path::Component::CurDir
                )
            })
        {
            return Err(payload_invalid(format!(
                "{label} source path is not repository-contained"
            )));
        }
    }
    match (value.get("start"), value.get("end")) {
        (None, None) => {}
        (Some(start), Some(end)) => {
            let start = start
                .as_i64()
                .ok_or_else(|| payload_invalid(format!("{label} start is not an integer")))?;
            let end = end
                .as_i64()
                .ok_or_else(|| payload_invalid(format!("{label} end is not an integer")))?;
            if start < 0 || end < start || value.get("file").is_none() {
                return Err(payload_invalid(format!(
                    "{label} has an invalid source range"
                )));
            }
        }
        _ => {
            return Err(payload_invalid(format!(
                "{label} has only one source range endpoint"
            )));
        }
    }
    if value.get("sourceProvenance").is_some()
        && value.get("sourceProvenance").and_then(Value::as_str)
            != Some("COMPILER_UTF16_RANGE_TO_UTF8_BYTES")
    {
        return Err(payload_invalid(format!(
            "{label} source provenance is invalid"
        )));
    }
    Ok(())
}

fn validate_optional_hashes(value: &Value, fields: &[&str], label: &str) -> Result<(), ClewError> {
    for field in fields {
        if let Some(hash) = value.get(*field)
            && hash.as_str().is_none_or(|hash| !is_canonical_sha256(hash))
        {
            return Err(payload_invalid(format!(
                "{label} {field} is not a canonical SHA-256 identity"
            )));
        }
    }
    Ok(())
}

pub(crate) fn validate_declaration_descriptor_boundary(value: &Value) -> Result<(), ClewError> {
    closed_payload(
        value,
        &[
            "schema",
            "file",
            "start",
            "end",
            "symbolIdentity",
            "stage",
            "code",
            "resolution",
            "provider",
            "module",
            "sourceSet",
            "sourceProvenance",
            "compilerAuthority",
            "rawRowHash",
            "retainedDescriptorHash",
        ],
        "declaration descriptor boundary",
    )?;
    if value.get("schema").and_then(Value::as_str) != Some("declaration-descriptor-boundary/0.1")
        || value.get("resolution").and_then(Value::as_str) != Some("UNKNOWN")
        || !matches!(
            value.get("provider").and_then(Value::as_str),
            Some(
                "K2_FIR"
                    | "COMPILER_DESCRIPTOR_NORMALIZER"
                    | "CODECLEW_DESCRIPTOR_NORMALIZER"
                    | "WORKER"
            )
        )
        || !matches!(
            value.get("stage").and_then(Value::as_str),
            Some("DECLARATION" | "CONSTRUCTOR_DECLARATION" | "NORMALIZE" | "ANALYSIS")
        )
        || !matches!(
            value.get("code").and_then(Value::as_str),
            Some(
                "GENERATED_OR_NO_SOURCE"
                    | "LOCAL_DECLARATION_UNSUPPORTED"
                    | "LOCAL_GENERATED_OR_NO_SOURCE"
                    | "UNRESOLVED_DESCRIPTOR_BOUNDARY"
                    | "NO_COMPILER_CALLABLE_ID"
                    | "LOCAL_CONSTRUCTOR_UNSUPPORTED"
                    | "UNRESOLVED_CONSTRUCTOR_DESCRIPTOR"
                    | "INCOMPLETE_COMPILER_DESCRIPTOR"
                    | "MALFORMED_COMPILER_FACT_ROW"
                    | "INVALID_DESCRIPTOR_SOURCE_PATH"
                    | "INVALID_DESCRIPTOR_IDENTITY"
                    | "INVALID_JVM_DESCRIPTOR"
                    | "UNKNOWN_DECLARATION_KIND"
                    | "UNKNOWN_VISIBILITY"
                    | "UNKNOWN_EFFECTIVE_VISIBILITY"
                    | "UNKNOWN_MODALITY"
                    | "DESCRIPTOR_SOURCE_NOT_IN_COMPILATION"
                    | "INVALID_DESCRIPTOR_SOURCE_RANGE"
                    | "UNRESOLVED_DESCRIPTOR_TYPE"
                    | "JVM_NAME_OVERRIDE_UNSUPPORTED"
                    | "SYNTAX_ONLY"
            )
        )
        || payload_string(value, "module").is_err()
        || payload_string(value, "sourceSet").is_err()
        || value.get("compilerAuthority").and_then(Value::as_str) != Some("fir-facts-extractor/0.6")
    {
        return Err(payload_invalid(
            "malformed declaration descriptor Unknown boundary",
        ));
    }
    validate_optional_boundary_location(value, "declaration descriptor boundary")?;
    validate_optional_hashes(
        value,
        &["rawRowHash", "retainedDescriptorHash"],
        "declaration descriptor boundary",
    )?;
    let compiler_normalized =
        value.get("provider").and_then(Value::as_str) == Some("COMPILER_DESCRIPTOR_NORMALIZER");
    if compiler_normalized && value.get("rawRowHash").is_none() {
        return Err(payload_invalid(
            "normalized descriptor boundary has no raw row authority",
        ));
    }
    if value.get("retainedDescriptorHash").is_some()
        && (!compiler_normalized
            || value.get("stage").and_then(Value::as_str) != Some("NORMALIZE")
            || !matches!(
                value.get("code").and_then(Value::as_str),
                Some(
                    "UNKNOWN_VISIBILITY"
                        | "UNKNOWN_EFFECTIVE_VISIBILITY"
                        | "UNKNOWN_MODALITY"
                        | "UNRESOLVED_DESCRIPTOR_TYPE"
                )
            ))
    {
        return Err(payload_invalid(
            "retained descriptor link is outside the partial-core contract",
        ));
    }
    Ok(())
}

pub(crate) fn validate_declaration_relation_boundary(value: &Value) -> Result<(), ClewError> {
    closed_payload(
        value,
        &[
            "schema",
            "file",
            "start",
            "end",
            "owner",
            "target",
            "relationKind",
            "stage",
            "code",
            "resolution",
            "provider",
            "sourceProvenance",
            "rawRowHash",
            "rawRowsHash",
            "affectedRowCount",
            "retainedRelationHash",
            "ownerIdentityHash",
            "rootFirKindHash",
            "nestedResolvedOccurrenceCount",
            "nestedResolvedOccurrenceKindHashes",
        ],
        "declaration relation boundary",
    )?;
    if value.get("schema").and_then(Value::as_str) != Some("declaration-relation-boundary/0.1")
        || value.get("resolution").and_then(Value::as_str) != Some("UNKNOWN")
        || !matches!(
            value.get("provider").and_then(Value::as_str),
            Some(
                "K2_FIR"
                    | "K2_FIR_CFG"
                    | "COMPILER_RELATION_NORMALIZER"
                    | "CODECLEW_RELATION_NORMALIZER"
                    | "WORKER"
            )
        )
        || payload_string(value, "stage").is_err()
        || payload_string(value, "code").is_err()
    {
        return Err(payload_invalid(
            "malformed declaration relation Unknown boundary",
        ));
    }
    validate_optional_boundary_location(value, "declaration relation boundary")?;
    for field in ["owner", "target", "relationKind"] {
        if value
            .get(field)
            .is_some_and(|field| field.as_str().is_none_or(str::is_empty))
        {
            return Err(payload_invalid(format!(
                "declaration relation boundary {field} is malformed"
            )));
        }
    }
    validate_optional_hashes(
        value,
        &[
            "rawRowHash",
            "rawRowsHash",
            "retainedRelationHash",
            "ownerIdentityHash",
            "rootFirKindHash",
        ],
        "declaration relation boundary",
    )?;
    if let Some(hashes) = value.get("nestedResolvedOccurrenceKindHashes") {
        let hashes = hashes.as_array().ok_or_else(|| {
            payload_invalid("return-value boundary occurrence hashes are not an array")
        })?;
        if hashes
            .iter()
            .any(|hash| hash.as_str().is_none_or(|hash| !is_canonical_sha256(hash)))
        {
            return Err(payload_invalid(
                "return-value boundary occurrence hash is malformed",
            ));
        }
    }
    if value
        .get("nestedResolvedOccurrenceCount")
        .is_some_and(|count| count.as_u64().is_none())
        || value
            .get("affectedRowCount")
            .is_some_and(|count| count.as_u64().is_none())
    {
        return Err(payload_invalid(
            "declaration relation boundary count is malformed",
        ));
    }
    let provider = payload_string(value, "provider")?;
    let stage = payload_string(value, "stage")?;
    let code = payload_string(value, "code")?;
    if matches!(
        provider,
        "COMPILER_RELATION_NORMALIZER" | "CODECLEW_RELATION_NORMALIZER"
    ) {
        let (hash_field, valid_code) = if provider == "COMPILER_RELATION_NORMALIZER" {
            (
                "rawRowHash",
                matches!(
                    code,
                    "INCOMPLETE_COMPILER_RELATION"
                        | "MALFORMED_COMPILER_FACT_ROW"
                        | "INVALID_RELATION_SOURCE_PATH"
                        | "INVALID_RELATION_IDENTITY"
                        | "UNKNOWN_RELATION_KIND"
                        | "REFERENCE_TO_QUARANTINED_DESCRIPTOR"
                        | "RELATION_SOURCE_NOT_IN_COMPILATION"
                        | "INVALID_RELATION_SOURCE_RANGE"
                        | "INVALID_RELATION_POSITIONAL_COORDINATE"
                        | "UNRESOLVED_RELATION_TYPE"
                ),
            )
        } else {
            (
                "rawRowsHash",
                matches!(
                    code,
                    "ARGUMENT_MAPPING_UNAVAILABLE"
                        | "NULL_COALESCING_FLOW_UNAVAILABLE"
                        | "RETURN_VALUE_FLOW_UNAVAILABLE"
                ),
            )
        };
        if !valid_code
            || value.get(hash_field).is_none()
            || (hash_field == "rawRowsHash"
                && value
                    .get("affectedRowCount")
                    .and_then(Value::as_u64)
                    .is_none_or(|count| count == 0))
        {
            return Err(payload_invalid(
                "normalized declaration relation boundary is incomplete",
            ));
        }
    }
    if (stage == "COORDINATE_NORMALIZATION" || code == "INVALID_RELATION_POSITIONAL_COORDINATE")
        && (provider != "COMPILER_RELATION_NORMALIZER"
            || stage != "COORDINATE_NORMALIZATION"
            || code != "INVALID_RELATION_POSITIONAL_COORDINATE")
    {
        return Err(payload_invalid(
            "relation coordinate boundary stage and code are inconsistent",
        ));
    }
    if value.get("retainedRelationHash").is_some()
        && (provider != "COMPILER_RELATION_NORMALIZER"
            || stage != "NORMALIZE"
            || code != "UNRESOLVED_RELATION_TYPE")
    {
        return Err(payload_invalid(
            "retained relation link is outside the partial-core contract",
        ));
    }
    if stage == "RETURN_VALUE"
        && !matches!(
            code,
            "IMPLICIT_RETURN_UNSUPPORTED"
                | "IMPLICIT_OR_MISSING_RETURN_SOURCE"
                | "UNRESOLVED_RETURN_OWNER"
                | "LOCAL_OR_GENERATED_RETURN_OWNER"
                | "RETURN_TARGET_IDENTITY_MISMATCH"
                | "NON_LINEAR_OR_MULTIPLE_RETURN_FLOW"
                | "RETURN_VALUE_NOT_DIRECT_RESOLVED_READ_OR_CALL"
                | "MULTIPLE_OR_AMBIGUOUS_RETURN_VALUE_OCCURRENCES"
                | "LOCAL_GENERATED_OR_UNRESOLVED_RETURN_VALUE"
                | "MISSING_RETURN_CFG"
                | "AMBIGUOUS_RETURN_CFG_NODE"
                | "RETURN_VALUE_CFG_PROOF_UNAVAILABLE"
        )
    {
        return Err(payload_invalid("unknown typed return-value boundary code"));
    }
    Ok(())
}

pub(crate) fn validate_kotlin_semantic_payload(
    value: &Value,
) -> Result<KotlinSemanticPayloadKind, ClewError> {
    match value.get("schema").and_then(Value::as_str) {
        Some("declaration-descriptor/0.1") => {
            validate_declaration_descriptor_fact(value)?;
            Ok(KotlinSemanticPayloadKind::DeclarationDescriptor)
        }
        Some("declaration-relation/0.1") => {
            validate_declaration_relation_fact(value)?;
            Ok(KotlinSemanticPayloadKind::DeclarationRelation)
        }
        Some("declaration-descriptor-boundary/0.1") => {
            validate_declaration_descriptor_boundary(value)?;
            Ok(KotlinSemanticPayloadKind::DeclarationDescriptorBoundary)
        }
        Some("declaration-relation-boundary/0.1") => {
            validate_declaration_relation_boundary(value)?;
            Ok(KotlinSemanticPayloadKind::DeclarationRelationBoundary)
        }
        Some(_) => Err(payload_invalid(
            "unsupported Kotlin semantic payload schema",
        )),
        None => Err(payload_invalid("Kotlin semantic payload has no schema")),
    }
}

pub(crate) fn validate_declaration_relation_snapshot(
    facts: &Value,
) -> Result<DeclarationRelationSnapshot, ClewError> {
    fn invalid(message: impl Into<String>) -> ClewError {
        ClewError::new(ErrorCode::InvalidInput, message)
    }
    fn staged(mut error: ClewError, stage: &str) -> ClewError {
        error.evidence.push(format!("verified-index-stage:{stage}"));
        error
    }
    fn required_string<'a>(value: &'a Value, field: &str) -> Result<&'a str, ClewError> {
        value
            .get(field)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| invalid(format!("declaration relation graph has no {field}")))
    }
    fn canonical_rows(rows: &[Value], label: &str) -> Result<(), ClewError> {
        let encoded = rows
            .iter()
            .map(canonical::bytes)
            .collect::<Result<Vec<_>, _>>()
            .map_err(internal)?;
        if encoded.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(invalid(format!(
                "declaration relation {label} must be canonical, sorted, and unique"
            )));
        }
        Ok(())
    }
    fn validate_occurrence(
        occurrence: &Value,
        label: &str,
        expected_nullable: bool,
    ) -> Result<(i64, i64), ClewError> {
        let start = occurrence
            .get("start")
            .and_then(Value::as_i64)
            .ok_or_else(|| invalid(format!("{label} has no source start")))?;
        let end = occurrence
            .get("end")
            .and_then(Value::as_i64)
            .ok_or_else(|| invalid(format!("{label} has no source end")))?;
        let rendered = required_string(occurrence, "type")?;
        let nullable = occurrence
            .get("nullable")
            .and_then(Value::as_bool)
            .ok_or_else(|| invalid(format!("{label} has no nullability")))?;
        if start < 0
            || end <= start
            || nullable != expected_nullable
            || nullable != rendered.trim_end().ends_with('?')
            || rendered.contains("..")
            || rendered.contains('!')
            || rendered.contains("<ERROR")
        {
            return Err(invalid(format!(
                "{label} source range, type, or nullability is inconsistent"
            )));
        }
        Ok((start, end))
    }
    fn rendered_nullability(rendered: &str) -> Result<bool, ClewError> {
        if rendered.contains("..")
            || rendered.contains('!')
            || rendered.contains("<ERROR")
            || rendered.contains("<unknown>")
            || rendered.contains("UNKNOWN")
        {
            return Err(invalid(
                "return-value relation contains an unresolved compiler type",
            ));
        }
        Ok(rendered.trim_end().ends_with('?'))
    }
    fn occurrence_range_and_node(
        occurrence: &Value,
        label: &str,
    ) -> Result<(i64, i64, u64), ClewError> {
        let object = occurrence
            .as_object()
            .ok_or_else(|| invalid(format!("{label} is not an object")))?;
        if object.len() != 3
            || !object.contains_key("start")
            || !object.contains_key("end")
            || !object.contains_key("cfgNodeId")
        {
            return Err(invalid(format!(
                "{label} has fields outside its exact contract"
            )));
        }
        let start = occurrence
            .get("start")
            .and_then(Value::as_i64)
            .ok_or_else(|| invalid(format!("{label} has no source start")))?;
        let end = occurrence
            .get("end")
            .and_then(Value::as_i64)
            .ok_or_else(|| invalid(format!("{label} has no source end")))?;
        let node = occurrence
            .get("cfgNodeId")
            .and_then(Value::as_u64)
            .ok_or_else(|| invalid(format!("{label} has no CFG node identity")))?;
        if start < 0 || end <= start {
            return Err(invalid(format!("{label} has an invalid source range")));
        }
        Ok((start, end, node))
    }
    fn validate_return_value_relation(
        relation: &Value,
        relations: &[Value],
        descriptors: &[Value],
    ) -> Result<(), ClewError> {
        let allowed = BTreeSet::from([
            "schema",
            "file",
            "start",
            "end",
            "kind",
            "owner",
            "target",
            "resolution",
            "provider",
            "sourceKind",
            "sourceOccurrence",
            "returnOccurrence",
            "resultOccurrence",
            "resultType",
            "resultNullable",
            "valueProvenance",
            "cfgProvenance",
            "evaluationCount",
            "orderKey",
            "cfgNodeIds",
            "sourceProvenance",
            "orderProvenance",
        ]);
        let object = relation
            .as_object()
            .ok_or_else(|| invalid("return-value relation is not an object"))?;
        if object.keys().any(|field| !allowed.contains(field.as_str()))
            || object.values().any(|value| match value {
                Value::String(value) => {
                    value.is_empty() || value == "UNKNOWN" || value.contains("<unknown>")
                }
                Value::Null => true,
                _ => false,
            })
        {
            return Err(invalid(
                "return-value PROVEN relation has unknown or out-of-contract fields",
            ));
        }
        let owner = required_string(relation, "owner")?;
        let target = required_string(relation, "target")?;
        let file = required_string(relation, "file")?;
        let source_kind = required_string(relation, "sourceKind")?;
        let source_relation_kind = match source_kind {
            "PROPERTY_READ" => "READS",
            "FUNCTION_CALL_RESULT" => "CALLS",
            _ => return Err(invalid("unknown return-value sourceKind")),
        };
        let source_occurrence = relation
            .get("sourceOccurrence")
            .ok_or_else(|| invalid("return-value relation has no source occurrence"))?;
        let return_occurrence = relation
            .get("returnOccurrence")
            .ok_or_else(|| invalid("return-value relation has no return occurrence"))?;
        let result_occurrence = relation
            .get("resultOccurrence")
            .ok_or_else(|| invalid("return-value relation has no result occurrence"))?;
        let (source_start, source_end, source_node) =
            occurrence_range_and_node(source_occurrence, "return-value source occurrence")?;
        let (return_start, return_end, return_node) =
            occurrence_range_and_node(return_occurrence, "return occurrence")?;
        let (result_start, result_end, result_node) =
            occurrence_range_and_node(result_occurrence, "return result occurrence")?;
        let start = relation.get("start").and_then(Value::as_i64).unwrap_or(-1);
        let end = relation.get("end").and_then(Value::as_i64).unwrap_or(-1);
        if (source_start, source_end, source_node) != (result_start, result_end, result_node)
            || start != return_start
            || end != return_end
            || return_start > source_start
            || source_end > return_end
            || relation.get("orderKey").and_then(Value::as_i64) != Some(source_start)
            || relation.get("valueProvenance").and_then(Value::as_str)
                != Some("FIR_RETURN_RESULT_IDENTITY")
            || relation.get("evaluationCount").and_then(Value::as_u64) != Some(1)
            || relation.get("orderProvenance").and_then(Value::as_str) != Some("K2_FIR_CFG")
        {
            return Err(invalid(
                "return-value occurrence ranges, identity, order, or evaluation are inconsistent",
            ));
        }
        let cfg_nodes = relation
            .get("cfgNodeIds")
            .and_then(Value::as_array)
            .ok_or_else(|| invalid("return-value relation has no CFG node set"))?;
        let cfg_node_values = cfg_nodes
            .iter()
            .map(|value| {
                value
                    .as_u64()
                    .ok_or_else(|| invalid("return-value CFG node identity is not an integer"))
            })
            .collect::<Result<BTreeSet<_>, _>>()?;
        if cfg_node_values.len() != cfg_nodes.len()
            || !cfg_node_values.contains(&source_node)
            || !cfg_node_values.contains(&return_node)
        {
            return Err(invalid(
                "return-value relation does not contain its exact unique CFG nodes",
            ));
        }
        let cfg = relation
            .get("cfgProvenance")
            .and_then(Value::as_object)
            .ok_or_else(|| invalid("return-value relation has no CFG provenance"))?;
        let expected_cfg_fields = BTreeSet::from([
            "graphName",
            "sourceReachesReturn",
            "sourceDominatesReturn",
            "sourceNodeKind",
            "returnNodeKind",
        ]);
        if cfg.len() != expected_cfg_fields.len()
            || cfg
                .keys()
                .any(|field| !expected_cfg_fields.contains(field.as_str()))
            || cfg
                .get("graphName")
                .and_then(Value::as_str)
                .is_none_or(str::is_empty)
            || cfg.get("sourceReachesReturn").and_then(Value::as_bool) != Some(true)
            || cfg.get("sourceDominatesReturn").and_then(Value::as_bool) != Some(true)
            || cfg.get("returnNodeKind").and_then(Value::as_str) != Some("JumpNode")
            || cfg.get("sourceNodeKind").and_then(Value::as_str)
                != Some(if source_kind == "PROPERTY_READ" {
                    "QualifiedAccessNode"
                } else {
                    "FunctionCallExitNode"
                })
        {
            return Err(invalid(
                "return-value CFG reachability or dominance provenance is incomplete",
            ));
        }
        let result_type = required_string(relation, "resultType")?;
        let result_nullable = relation
            .get("resultNullable")
            .and_then(Value::as_bool)
            .ok_or_else(|| invalid("return-value relation has no result nullability"))?;
        if rendered_nullability(result_type)? != result_nullable {
            return Err(invalid(
                "return-value result type and nullability are inconsistent",
            ));
        }
        let owner_descriptors = descriptors
            .iter()
            .filter(|descriptor| {
                descriptor.get("declarationKind").and_then(Value::as_str) == Some("FUNCTION")
                    && descriptor.get("compilerCallableId").and_then(Value::as_str) == Some(owner)
            })
            .collect::<Vec<_>>();
        let source_descriptors = descriptors
            .iter()
            .filter(|descriptor| {
                matches!(
                    (
                        source_kind,
                        descriptor.get("declarationKind").and_then(Value::as_str)
                    ),
                    ("PROPERTY_READ", Some("PROPERTY" | "MUTABLE_PROPERTY"))
                        | ("FUNCTION_CALL_RESULT", Some("FUNCTION"))
                ) && descriptor.get("compilerCallableId").and_then(Value::as_str) == Some(target)
            })
            .collect::<Vec<_>>();
        let ([owner_descriptor], [source_descriptor]) =
            (owner_descriptors.as_slice(), source_descriptors.as_slice())
        else {
            return Err(staged(
                invalid("return-value owner or source descriptor is missing or ambiguous"),
                "CROSS_GRAPH_CONSISTENCY",
            ));
        };
        let (source_type, source_nullable) = if source_kind == "PROPERTY_READ" {
            (
                required_string(source_descriptor, "declaredType")?,
                source_descriptor
                    .get("declaredNullable")
                    .and_then(Value::as_bool),
            )
        } else {
            (
                required_string(source_descriptor, "returnType")?,
                source_descriptor
                    .get("returnNullable")
                    .and_then(Value::as_bool),
            )
        };
        if source_nullable != Some(result_nullable)
            || source_type != result_type
            || required_string(owner_descriptor, "returnType")? != result_type
            || owner_descriptor
                .get("returnNullable")
                .and_then(Value::as_bool)
                != Some(result_nullable)
        {
            return Err(staged(
                invalid("return-value descriptors disagree with the exact compiler result type"),
                "CROSS_GRAPH_CONSISTENCY",
            ));
        }
        let source_rows = relations
            .iter()
            .filter(|candidate| {
                candidate.get("kind").and_then(Value::as_str) == Some(source_relation_kind)
                    && candidate.get("owner").and_then(Value::as_str) == Some(owner)
                    && candidate.get("target").and_then(Value::as_str) == Some(target)
                    && candidate.get("file").and_then(Value::as_str) == Some(file)
                    && candidate.get("start").and_then(Value::as_i64) == Some(source_start)
                    && candidate.get("end").and_then(Value::as_i64) == Some(source_end)
            })
            .collect::<Vec<_>>();
        let [source_row] = source_rows.as_slice() else {
            return Err(staged(
                invalid("return-value source READS/CALLS occurrence is missing or ambiguous"),
                "CROSS_GRAPH_CONSISTENCY",
            ));
        };
        if required_string(source_row, "resultType")? != result_type
            || source_row
                .get("cfgNodeIds")
                .and_then(Value::as_array)
                .is_none_or(|nodes| !nodes.iter().any(|node| node.as_u64() == Some(source_node)))
            || source_row.get("resolution").and_then(Value::as_str) != Some("PROVEN")
            || source_row.get("provider").and_then(Value::as_str) != Some("K2_FIR")
        {
            return Err(staged(
                invalid("return-value source row type or CFG identity is inconsistent"),
                "CROSS_GRAPH_CONSISTENCY",
            ));
        }
        Ok(())
    }

    let graph = facts
        .get("declarationRelations")
        .filter(|value| value.is_object())
        .ok_or_else(|| invalid("worker index has no declaration relation graph"))?;
    if graph.get("schema").and_then(Value::as_str) != Some("declaration-relation-graph/0.1") {
        return Err(invalid("unsupported declaration relation graph schema"));
    }
    let compilation = required_string(graph, "compilation")?;
    if facts.get("compilation").and_then(Value::as_str) != Some(compilation) {
        return Err(staged(
            ClewError::new(
                ErrorCode::ProjectModelChanged,
                "declaration relation graph compilation differs from worker index snapshot",
            ),
            "SOURCE_BINDING",
        ));
    }
    let coverage = graph.get("coverage").and_then(Value::as_str);
    if !matches!(coverage, Some("COMPLETE_SUPPORTED_SUBSET" | "PARTIAL")) {
        return Err(invalid("unknown declaration relation coverage status"));
    }
    let relations = graph
        .get("relations")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("declaration relation graph has no relations"))?;
    let descriptor_rows = facts
        .pointer("/declarationDescriptors/descriptors")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            staged(
                invalid("declaration relations have no authoritative descriptor graph"),
                "CROSS_GRAPH_CONSISTENCY",
            )
        })?;
    let source_binding =
        declaration_source_binding(facts).map_err(|error| staged(error, "SOURCE_BINDING"))?;
    let source_compilations = source_binding["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| {
            let path = required_string(file, "path")?;
            let module = required_string(file, "module")?;
            let source_set = required_string(file, "sourceSet")?;
            let identity = format!("{module}/{source_set}");
            if identity != compilation {
                return Err(ClewError::new(
                    ErrorCode::ProjectModelChanged,
                    "declaration relation source module/sourceSet differs from compilation",
                ));
            }
            Ok((path.to_owned(), (module.to_owned(), source_set.to_owned())))
        })
        .collect::<Result<BTreeMap<_, _>, ClewError>>()?;
    canonical_rows(relations, "rows")?;
    let mut partial_relations = BTreeMap::new();
    for relation in relations {
        if validate_kotlin_semantic_payload(relation)?
            != KotlinSemanticPayloadKind::DeclarationRelation
        {
            return Err(invalid(
                "declaration relation row has the wrong payload kind",
            ));
        }
        if relation.get("schema").and_then(Value::as_str) != Some("declaration-relation/0.1")
            || relation.get("resolution").and_then(Value::as_str) != Some("PROVEN")
            || relation.get("provider").and_then(Value::as_str) != Some("K2_FIR")
        {
            return Err(invalid(
                "malformed or non-authoritative declaration relation",
            ));
        }
        if !matches!(
            relation.get("kind").and_then(Value::as_str),
            Some(
                "OVERRIDES"
                    | "CALLS"
                    | "REFERENCES"
                    | "CONSTRUCTS"
                    | "READS"
                    | "WRITES"
                    | "INITIALIZES"
                    | "NULL_COALESCES"
                    | "RETURNS_VALUE_FROM"
            )
        ) {
            return Err(invalid("unknown declaration relation kind"));
        }
        let has_partial_marker =
            relation.get("attributeCoverage").is_some() || relation.get("sourceRowHash").is_some();
        if has_partial_marker {
            if coverage != Some("PARTIAL")
                || relation.get("attributeCoverage").and_then(Value::as_str) != Some("PARTIAL")
            {
                return Err(invalid(
                    "partial declaration relation requires PARTIAL row and graph coverage",
                ));
            }
            if !matches!(
                relation.get("kind").and_then(Value::as_str),
                Some("CALLS" | "CONSTRUCTS")
            ) {
                return Err(invalid(
                    "partial declaration relation cannot retain a special flow kind",
                ));
            }
            let allowed = BTreeSet::from([
                "schema",
                "file",
                "start",
                "end",
                "kind",
                "owner",
                "target",
                "resolution",
                "provider",
                "cfgNodeIds",
                "sourceProvenance",
                "orderProvenance",
                "attributeCoverage",
                "sourceRowHash",
                "targetCompilerCallableId",
                "targetJvmDescriptor",
            ]);
            let object = relation
                .as_object()
                .ok_or_else(|| invalid("partial declaration relation is not an object"))?;
            if object.keys().any(|field| !allowed.contains(field.as_str())) {
                return Err(invalid(
                    "partial declaration relation has an unexpected or typed field",
                ));
            }
            let cfg_nodes = relation
                .get("cfgNodeIds")
                .and_then(Value::as_array)
                .ok_or_else(|| invalid("partial declaration relation has no CFG node set"))?;
            let mut previous_cfg_node = None;
            for cfg_node in cfg_nodes {
                let cfg_node = cfg_node.as_u64().ok_or_else(|| {
                    invalid("partial declaration relation CFG node is not a nonnegative integer")
                })?;
                if previous_cfg_node.is_some_and(|previous| previous >= cfg_node) {
                    return Err(invalid(
                        "partial declaration relation CFG nodes are not canonical and unique",
                    ));
                }
                previous_cfg_node = Some(cfg_node);
            }
            let source_row_hash = required_string(relation, "sourceRowHash")?;
            if !is_canonical_sha256(source_row_hash) {
                return Err(invalid(
                    "partial declaration relation source row hash is malformed",
                ));
            }
            let retained_hash = canonical::hash(relation).map_err(internal)?;
            if partial_relations
                .insert(source_row_hash.to_owned(), (retained_hash, relation))
                .is_some()
            {
                return Err(invalid(
                    "partial declaration relations repeat a source row link",
                ));
            }
        }
        let file = required_string(relation, "file")?;
        if Path::new(file).is_absolute()
            || Path::new(file)
                .components()
                .any(|part| matches!(part, std::path::Component::ParentDir))
            || required_string(relation, "owner").is_err()
            || required_string(relation, "target").is_err()
        {
            return Err(invalid(
                "declaration relation identity is not repository-contained",
            ));
        }
        let start = relation.get("start").and_then(Value::as_i64).unwrap_or(-1);
        let end = relation.get("end").and_then(Value::as_i64).unwrap_or(-1);
        if start < 0
            || end < start
            || !relation.get("cfgNodeIds").is_some_and(Value::is_array)
            || !source_compilations.contains_key(file)
            || relation.get("sourceProvenance").and_then(Value::as_str)
                != Some("COMPILER_UTF16_RANGE_TO_UTF8_BYTES")
            || !matches!(
                relation.get("orderProvenance").and_then(Value::as_str),
                Some("K2_FIR_CFG" | "FIR_SOURCE_RANGE" | "UNKNOWN")
            )
        {
            return Err(invalid(
                "declaration relation has invalid source/CFG provenance",
            ));
        }
        if let Some(arguments) = relation.get("argumentToParameter") {
            let arguments = arguments
                .as_array()
                .ok_or_else(|| invalid("argument mapping is not an array"))?;
            let mut indices = BTreeSet::new();
            for argument in arguments {
                let index = argument
                    .get("parameterIndex")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| invalid("argument mapping has no nonnegative parameterIndex"))?;
                if !indices.insert(index) {
                    return Err(invalid("argument mapping repeats a parameterIndex"));
                }
                required_string(argument, "parameterType")?;
            }
            let relation_kind = relation.get("kind").and_then(Value::as_str);
            if matches!(relation_kind, Some("CALLS" | "CONSTRUCTS")) {
                let raw_target = required_string(relation, "target")?;
                let exact_target = match (
                    relation
                        .get("targetCompilerCallableId")
                        .and_then(Value::as_str),
                    relation.get("targetJvmDescriptor").and_then(Value::as_str),
                ) {
                    (Some(callable), Some(descriptor)) => Some((callable, descriptor)),
                    (None, None) => None,
                    _ => {
                        return Err(staged(
                            invalid("call target exact identity fields are incomplete"),
                            "CROSS_GRAPH_CONSISTENCY",
                        ));
                    }
                };
                let expected_kind = if relation_kind == Some("CALLS") {
                    "FUNCTION"
                } else {
                    "CONSTRUCTOR"
                };
                let descriptors = descriptor_rows
                    .iter()
                    .filter(|descriptor| {
                        descriptor.get("declarationKind").and_then(Value::as_str)
                            == Some(expected_kind)
                            && match exact_target {
                                Some((callable, jvm_descriptor)) => {
                                    descriptor.get("symbolIdentity").and_then(Value::as_str)
                                        == Some(raw_target)
                                        && descriptor
                                            .get("compilerCallableId")
                                            .and_then(Value::as_str)
                                            == Some(callable)
                                        && descriptor.get("jvmDescriptor").and_then(Value::as_str)
                                            == Some(jvm_descriptor)
                                }
                                None => {
                                    descriptor.get("compilerCallableId").and_then(Value::as_str)
                                        == Some(raw_target)
                                }
                            }
                    })
                    .collect::<Vec<_>>();
                if descriptors.len() != 1 {
                    return Err(staged(
                        invalid(
                            "call/constructor argument mapping target is missing or overload-ambiguous",
                        ),
                        "CROSS_GRAPH_CONSISTENCY",
                    ));
                }
                let parameters = descriptors[0]
                    .get("parameterTypes")
                    .and_then(Value::as_array)
                    .ok_or_else(|| {
                        staged(
                            invalid("call target descriptor has no parameter types"),
                            "CROSS_GRAPH_CONSISTENCY",
                        )
                    })?;
                if let Some((_, exact_jvm_descriptor)) = exact_target {
                    let omitted = relation
                        .get("omittedDefaultParameterIndices")
                        .and_then(Value::as_array)
                        .ok_or_else(|| {
                            staged(
                                invalid("exact call relation has no omitted-default set"),
                                "CROSS_GRAPH_CONSISTENCY",
                            )
                        })?;
                    let omitted = omitted
                        .iter()
                        .map(|index| {
                            index.as_u64().ok_or_else(|| {
                                staged(
                                    invalid("omitted-default parameter index is invalid"),
                                    "CROSS_GRAPH_CONSISTENCY",
                                )
                            })
                        })
                        .collect::<Result<BTreeSet<_>, _>>()?;
                    if parameters.len()
                        != parse_jvm_method_descriptor(exact_jvm_descriptor)?.parameter_count
                    {
                        return Err(staged(
                            invalid(
                                "target descriptor parameter slots disagree with JVM descriptor",
                            ),
                            "CROSS_GRAPH_CONSISTENCY",
                        ));
                    }
                    for index in &omitted {
                        let slot = parameters.get(*index as usize).ok_or_else(|| {
                            staged(
                                invalid("omitted-default parameter is outside target descriptor"),
                                "CROSS_GRAPH_CONSISTENCY",
                            )
                        })?;
                        if slot.get("hasDefault").and_then(Value::as_bool) != Some(true) {
                            return Err(staged(
                                invalid(
                                    "omitted parameter lacks compiler-confirmed default authority",
                                ),
                                "CROSS_GRAPH_CONSISTENCY",
                            ));
                        }
                    }
                    if (0..parameters.len() as u64)
                        .any(|index| !indices.contains(&index) && !omitted.contains(&index))
                    {
                        return Err(staged(
                            invalid(
                                "mapped and compiler-defaulted parameters do not cover the target",
                            ),
                            "CROSS_GRAPH_CONSISTENCY",
                        ));
                    }
                }
                for argument in arguments {
                    let index = argument["parameterIndex"].as_u64().unwrap() as usize;
                    let slot = parameters.get(index).ok_or_else(|| {
                        staged(
                            invalid("argument mapping parameterIndex is outside target descriptor"),
                            "CROSS_GRAPH_CONSISTENCY",
                        )
                    })?;
                    if required_string(argument, "parameterType")? != required_string(slot, "type")?
                    {
                        return Err(staged(
                            invalid("argument mapping parameterType differs from descriptor slot"),
                            "CROSS_GRAPH_CONSISTENCY",
                        ));
                    }
                    let argument_start = argument
                        .get("argumentStart")
                        .and_then(Value::as_i64)
                        .ok_or_else(|| invalid("argument mapping has no source occurrence"))?;
                    let argument_end = argument.get("argumentEnd").and_then(Value::as_i64);
                    if argument_start < start
                        || argument_start >= end
                        || argument_end.is_some_and(|argument_end| {
                            argument_start >= argument_end || argument_end > end
                        })
                        || exact_target.is_some() && argument_end.is_none()
                    {
                        return Err(invalid(
                            "argument mapping source occurrence is outside the call range",
                        ));
                    }
                }
            }
        }
        if relation.get("kind").and_then(Value::as_str) == Some("NULL_COALESCES") {
            let source_target = required_string(relation, "sourceTarget")?;
            let fallback_target = required_string(relation, "fallbackTarget")?;
            if required_string(relation, "target")? != fallback_target
                || source_target == fallback_target
                || relation.get("orderProvenance").and_then(Value::as_str) != Some("K2_FIR_CFG")
                || relation
                    .get("cfgNodeIds")
                    .and_then(Value::as_array)
                    .is_none_or(Vec::is_empty)
            {
                return Err(invalid(
                    "null coalescing relation lacks exact compiler/CFG authority",
                ));
            }
            let (lhs_start, lhs_end) = validate_occurrence(
                relation
                    .get("sourceOccurrence")
                    .ok_or_else(|| invalid("null coalescing relation has no source occurrence"))?,
                "null coalescing source occurrence",
                true,
            )?;
            let (fallback_start, fallback_end) = validate_occurrence(
                relation.get("fallbackOccurrence").ok_or_else(|| {
                    invalid("null coalescing relation has no fallback occurrence")
                })?,
                "null coalescing fallback occurrence",
                false,
            )?;
            let (merged_start, merged_end) = validate_occurrence(
                relation
                    .get("mergedOccurrence")
                    .ok_or_else(|| invalid("null coalescing relation has no merged occurrence"))?,
                "null coalescing merged occurrence",
                false,
            )?;
            let branch = relation
                .get("branchProvenance")
                .filter(|value| value.is_object())
                .ok_or_else(|| invalid("null coalescing relation has no branch provenance"))?;
            if branch.get("kind").and_then(Value::as_str) != Some("FIR_ELVIS_EXPRESSION")
                || branch.get("nullableBranchStart").and_then(Value::as_i64) != Some(lhs_start)
                || branch.get("fallbackBranchStart").and_then(Value::as_i64) != Some(fallback_start)
                || branch.get("mergeStart").and_then(Value::as_i64) != Some(merged_start)
                || branch.get("mergeEnd").and_then(Value::as_i64) != Some(merged_end)
                || merged_start != start
                || merged_end != end
                || lhs_start < merged_start
                || lhs_end > merged_end
                || fallback_start < merged_start
                || fallback_end > merged_end
                || lhs_end > fallback_start
                || relation.get("orderKey").and_then(Value::as_i64) != Some(merged_start)
            {
                return Err(invalid(
                    "null coalescing branch/merge ranges are inconsistent",
                ));
            }
        }
        if relation.get("kind").and_then(Value::as_str) == Some("RETURNS_VALUE_FROM") {
            validate_return_value_relation(relation, relations, descriptor_rows)?;
        }
    }

    let boundaries = graph
        .get("boundaries")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("declaration relation graph has no boundaries"))?;
    canonical_rows(boundaries, "boundaries")?;
    let mut paired_partial_relations = BTreeSet::new();
    for boundary in boundaries {
        if validate_kotlin_semantic_payload(boundary)?
            != KotlinSemanticPayloadKind::DeclarationRelationBoundary
        {
            return Err(invalid(
                "declaration relation boundary has the wrong payload kind",
            ));
        }
        if boundary.get("schema").and_then(Value::as_str)
            != Some("declaration-relation-boundary/0.1")
            || boundary.get("resolution").and_then(Value::as_str) != Some("UNKNOWN")
            || !matches!(
                boundary.get("provider").and_then(Value::as_str),
                Some(
                    "K2_FIR"
                        | "K2_FIR_CFG"
                        | "COMPILER_RELATION_NORMALIZER"
                        | "CODECLEW_RELATION_NORMALIZER"
                        | "WORKER"
                )
            )
            || required_string(boundary, "stage").is_err()
            || required_string(boundary, "code").is_err()
        {
            return Err(invalid("malformed declaration relation Unknown boundary"));
        }
        if matches!(
            boundary.get("provider").and_then(Value::as_str),
            Some("COMPILER_RELATION_NORMALIZER" | "CODECLEW_RELATION_NORMALIZER")
        ) {
            let hash_field = if boundary.get("provider").and_then(Value::as_str)
                == Some("CODECLEW_RELATION_NORMALIZER")
            {
                "rawRowsHash"
            } else {
                "rawRowHash"
            };
            let row_hash = required_string(boundary, hash_field)?;
            if !is_canonical_sha256(row_hash) {
                return Err(invalid("relation boundary raw row hash is malformed"));
            }
            if hash_field == "rawRowsHash"
                && boundary
                    .get("affectedRowCount")
                    .and_then(Value::as_u64)
                    .is_none_or(|count| count == 0)
            {
                return Err(invalid("relation boundary affected row count is malformed"));
            }
            let valid_code = match boundary.get("provider").and_then(Value::as_str) {
                Some("COMPILER_RELATION_NORMALIZER") => matches!(
                    boundary.get("code").and_then(Value::as_str),
                    Some(
                        "INCOMPLETE_COMPILER_RELATION"
                            | "MALFORMED_COMPILER_FACT_ROW"
                            | "INVALID_RELATION_SOURCE_PATH"
                            | "INVALID_RELATION_IDENTITY"
                            | "UNKNOWN_RELATION_KIND"
                            | "REFERENCE_TO_QUARANTINED_DESCRIPTOR"
                            | "RELATION_SOURCE_NOT_IN_COMPILATION"
                            | "INVALID_RELATION_SOURCE_RANGE"
                            | "INVALID_RELATION_POSITIONAL_COORDINATE"
                            | "UNRESOLVED_RELATION_TYPE"
                    )
                ),
                Some("CODECLEW_RELATION_NORMALIZER") => matches!(
                    boundary.get("code").and_then(Value::as_str),
                    Some(
                        "ARGUMENT_MAPPING_UNAVAILABLE"
                            | "NULL_COALESCING_FLOW_UNAVAILABLE"
                            | "RETURN_VALUE_FLOW_UNAVAILABLE"
                    )
                ),
                _ => false,
            };
            if !valid_code {
                return Err(invalid("unknown normalized relation boundary code"));
            }
        }
        if let Some(retained_hash) = boundary.get("retainedRelationHash") {
            let retained_hash = retained_hash.as_str().ok_or_else(|| {
                invalid("relation boundary retained relation hash is not a string")
            })?;
            if boundary.get("provider").and_then(Value::as_str)
                != Some("COMPILER_RELATION_NORMALIZER")
                || boundary.get("stage").and_then(Value::as_str) != Some("NORMALIZE")
                || boundary.get("code").and_then(Value::as_str) != Some("UNRESOLVED_RELATION_TYPE")
                || !is_canonical_sha256(retained_hash)
            {
                return Err(invalid(
                    "retained relation link is outside the partial-core contract",
                ));
            }
            let source_row_hash = required_string(boundary, "rawRowHash")?;
            let Some((expected_retained_hash, relation)) = partial_relations.get(source_row_hash)
            else {
                return Err(invalid(
                    "retained relation boundary has no matching partial relation",
                ));
            };
            if retained_hash != expected_retained_hash
                || boundary.get("file") != relation.get("file")
                || boundary.get("owner") != relation.get("owner")
                || boundary.get("target") != relation.get("target")
                || boundary.get("relationKind") != relation.get("kind")
                || boundary.get("start") != relation.get("start")
                || boundary.get("end") != relation.get("end")
            {
                return Err(invalid(
                    "retained relation boundary disagrees with its exact partial relation",
                ));
            }
            if !paired_partial_relations.insert(source_row_hash.to_owned()) {
                return Err(invalid(
                    "partial declaration relation has duplicate retained boundary links",
                ));
            }
        }
        if boundary.get("stage").and_then(Value::as_str) == Some("RETURN_VALUE") {
            let allowed_codes = BTreeSet::from([
                "IMPLICIT_RETURN_UNSUPPORTED",
                "IMPLICIT_OR_MISSING_RETURN_SOURCE",
                "UNRESOLVED_RETURN_OWNER",
                "LOCAL_OR_GENERATED_RETURN_OWNER",
                "RETURN_TARGET_IDENTITY_MISMATCH",
                "NON_LINEAR_OR_MULTIPLE_RETURN_FLOW",
                "RETURN_VALUE_NOT_DIRECT_RESOLVED_READ_OR_CALL",
                "MULTIPLE_OR_AMBIGUOUS_RETURN_VALUE_OCCURRENCES",
                "LOCAL_GENERATED_OR_UNRESOLVED_RETURN_VALUE",
                "MISSING_RETURN_CFG",
                "AMBIGUOUS_RETURN_CFG_NODE",
                "RETURN_VALUE_CFG_PROOF_UNAVAILABLE",
            ]);
            if boundary
                .get("code")
                .and_then(Value::as_str)
                .is_none_or(|code| !allowed_codes.contains(code))
            {
                return Err(invalid("unknown typed return-value boundary code"));
            }
            for digest in ["ownerIdentityHash", "rootFirKindHash"] {
                if let Some(value) = boundary.get(digest) {
                    let value = value
                        .as_str()
                        .ok_or_else(|| invalid("return-value diagnostic hash is not a string"))?;
                    if !value.starts_with("sha256:") || value.len() != 71 {
                        return Err(invalid("return-value diagnostic hash is malformed"));
                    }
                }
            }
            if let Some(count) = boundary.get("nestedResolvedOccurrenceCount") {
                count.as_u64().ok_or_else(|| {
                    invalid("return-value occurrence diagnostic count is not nonnegative")
                })?;
            }
            if let Some(kinds) = boundary.get("nestedResolvedOccurrenceKindHashes") {
                let kinds = kinds.as_array().ok_or_else(|| {
                    invalid("return-value occurrence kind diagnostics are not an array")
                })?;
                if kinds.iter().any(|value| {
                    value
                        .as_str()
                        .is_none_or(|value| !value.starts_with("sha256:") || value.len() != 71)
                }) {
                    return Err(invalid("return-value occurrence kind hash is malformed"));
                }
            }
        }
    }
    if paired_partial_relations.len() != partial_relations.len() {
        return Err(invalid(
            "partial declaration relation has no matching retained boundary link",
        ));
    }

    let provenance = graph
        .get("provenance")
        .filter(|value| value.is_object())
        .ok_or_else(|| invalid("declaration relation graph has no authority provenance"))?;
    if provenance.get("provider").and_then(Value::as_str) != Some("COMPILER_SEMANTIC_FACTS")
        || provenance.get("extractorSchema").and_then(Value::as_str)
            != Some("fir-facts-extractor/0.6")
        || required_string(provenance, "workerVersion").is_err()
        || required_string(provenance, "workerProtocolVersion").is_err()
    {
        return Err(invalid(
            "declaration relation authority provenance is incomplete",
        ));
    }
    let plugin = required_string(provenance, "pluginArtifactFingerprint")?;
    if !plugin.starts_with("sha256:") || plugin.len() != 71 {
        return Err(invalid(
            "declaration relation plugin fingerprint is malformed",
        ));
    }
    for (provenance_field, index_field) in [
        ("compilerVersion", "compilerVersion"),
        ("workerCompilerVersion", "compilerVersion"),
        ("projectModelHash", "projectModelHash"),
        ("classpathHash", "classpathHash"),
        ("compilerOptionsHash", "compilerOptionsHash"),
    ] {
        if required_string(provenance, provenance_field)?
            != facts
                .get(index_field)
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| invalid(format!("worker index has no {index_field}")))?
        {
            return Err(ClewError::new(
                ErrorCode::ProjectModelChanged,
                format!(
                    "declaration relation {provenance_field} differs from worker index snapshot"
                ),
            ));
        }
    }
    let hash = required_string(facts, "declarationRelationHash")?.to_owned();
    let computed = canonical::hash(graph).map_err(internal)?;
    if hash != computed {
        return Err(ClewError::new(
            ErrorCode::ProjectModelChanged,
            "declaration relation hash differs from canonical Rust graph hash",
        ));
    }
    Ok(DeclarationRelationSnapshot {
        graph: graph.clone(),
        hash,
        provenance: provenance.clone(),
    })
}

pub(crate) fn validate_declaration_descriptor_snapshot(
    facts: &Value,
) -> Result<DeclarationDescriptorSnapshot, ClewError> {
    fn invalid(message: impl Into<String>) -> ClewError {
        ClewError::new(ErrorCode::InvalidInput, message)
    }
    fn required_string<'a>(value: &'a Value, field: &str) -> Result<&'a str, ClewError> {
        value
            .get(field)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| invalid(format!("declaration descriptor graph has no {field}")))
    }
    fn canonical_rows(rows: &[Value], label: &str) -> Result<(), ClewError> {
        let encoded = rows
            .iter()
            .map(canonical::bytes)
            .collect::<Result<Vec<_>, _>>()
            .map_err(internal)?;
        if encoded.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(invalid(format!(
                "declaration descriptor {label} must be canonical, sorted, and unique"
            )));
        }
        Ok(())
    }
    fn safe_file(value: &Value) -> Result<(), ClewError> {
        let file = required_string(value, "file")?;
        let path = Path::new(file);
        if path.is_absolute()
            || path.components().any(|part| {
                !matches!(
                    part,
                    std::path::Component::Normal(_) | std::path::Component::CurDir
                )
            })
        {
            return Err(invalid(
                "declaration descriptor source path is not repository-contained",
            ));
        }
        Ok(())
    }
    fn nonempty_string_array(value: &Value, field: &str) -> Result<(), ClewError> {
        let items = value
            .get(field)
            .and_then(Value::as_array)
            .ok_or_else(|| invalid(format!("declaration descriptor has no {field}")))?;
        if items
            .iter()
            .any(|item| item.as_str().is_none_or(str::is_empty))
        {
            return Err(invalid(format!(
                "declaration descriptor {field} contains a non-string identity"
            )));
        }
        Ok(())
    }
    fn validate_type_parameters(value: &Value) -> Result<(), ClewError> {
        let parameters = value
            .get("typeParameters")
            .and_then(Value::as_array)
            .ok_or_else(|| invalid("declaration descriptor has no typeParameters"))?;
        for (index, parameter) in parameters.iter().enumerate() {
            if parameter.get("index").and_then(Value::as_u64) != Some(index as u64)
                || required_string(parameter, "compilerName").is_err()
            {
                return Err(invalid(
                    "declaration descriptor type parameter identity is invalid",
                ));
            }
            nonempty_string_array(parameter, "bounds")?;
            let bounds = parameter["bounds"].as_array().unwrap();
            let encoded = bounds.iter().map(Value::to_string).collect::<Vec<_>>();
            if encoded.windows(2).any(|pair| pair[0] >= pair[1]) {
                return Err(invalid(
                    "declaration descriptor type parameter bounds are not canonical",
                ));
            }
        }
        Ok(())
    }
    fn validate_typed_value(
        value: &Value,
        type_field: &str,
        null_field: &str,
    ) -> Result<(), ClewError> {
        let rendered = required_string(value, type_field)?;
        let nullable = value
            .get(null_field)
            .and_then(Value::as_bool)
            .ok_or_else(|| {
                invalid(format!(
                    "declaration descriptor has no boolean {null_field}"
                ))
            })?;
        if rendered.contains("..") || rendered.contains('!') || rendered.contains("<ERROR") {
            return Err(invalid(
                "flexible, platform, or unresolved compiler type cannot be PROVEN",
            ));
        }
        if nullable != rendered.trim_end().ends_with('?') {
            return Err(invalid(format!(
                "declaration descriptor {null_field} disagrees with compiler type rendering"
            )));
        }
        Ok(())
    }
    fn validate_parameter_slot(value: &Value, index: usize) -> Result<(), ClewError> {
        closed_payload(
            value,
            &["index", "type", "nullable", "hasDefault"],
            "declaration descriptor parameter slot",
        )?;
        if value.get("index").and_then(Value::as_u64) != Some(index as u64) {
            return Err(invalid("descriptor parameter indexes are not canonical"));
        }
        validate_typed_value(value, "type", "nullable")?;
        if value
            .get("hasDefault")
            .is_some_and(|flag| !flag.is_boolean())
        {
            return Err(invalid(
                "descriptor parameter hasDefault authority is not boolean",
            ));
        }
        Ok(())
    }
    fn validate_field_closure(value: &Value, kind: &str) -> Result<(), ClewError> {
        let mut allowed = BTreeSet::from([
            "schema",
            "file",
            "start",
            "end",
            "startLine",
            "endLine",
            "lineProvenance",
            "symbolIdentity",
            "declarationKind",
            "ownerIdentity",
            "containment",
            "visibility",
            "effectiveVisibility",
            "exportBoundary",
            "modality",
            "resolution",
            "provider",
            "module",
            "sourceSet",
            "sourceProvenance",
            "compilerAuthority",
            "typeParameters",
        ]);
        match kind {
            "FUNCTION" => allowed.extend([
                "compilerCallableId",
                "jvmDescriptor",
                "isOverride",
                "spring",
                "returnType",
                "returnNullable",
                "parameterTypes",
                "receiverType",
            ]),
            "CONSTRUCTOR" => allowed.extend([
                "compilerCallableId",
                "compilerClassId",
                "isPrimary",
                "jvmDescriptor",
                "parameterTypes",
            ]),
            "PROPERTY" | "MUTABLE_PROPERTY" => allowed.extend([
                "compilerCallableId",
                "isOverride",
                "declaredType",
                "declaredNullable",
            ]),
            "CLASS" => allowed.extend(["compilerClassId", "spring"]),
            _ => return Err(invalid("unknown declaration descriptor kind")),
        }
        let object = value
            .as_object()
            .ok_or_else(|| invalid("declaration descriptor row is not an object"))?;
        if object.keys().any(|field| !allowed.contains(field.as_str())) {
            return Err(invalid(
                "declaration descriptor has a field outside its kind-specific contract",
            ));
        }
        Ok(())
    }
    fn validate_partial_descriptor(value: &Value, kind: &str) -> Result<(), ClewError> {
        let mut allowed = BTreeSet::from([
            "schema",
            "file",
            "start",
            "end",
            "startLine",
            "endLine",
            "lineProvenance",
            "symbolIdentity",
            "declarationKind",
            "ownerIdentity",
            "containment",
            "resolution",
            "provider",
            "module",
            "sourceSet",
            "sourceProvenance",
            "compilerAuthority",
            "attributeCoverage",
            "sourceRowHash",
        ]);
        match kind {
            "FUNCTION" => allowed.extend(["compilerCallableId", "jvmDescriptor"]),
            "CONSTRUCTOR" => {
                allowed.extend(["compilerCallableId", "compilerClassId", "jvmDescriptor"])
            }
            "PROPERTY" | "MUTABLE_PROPERTY" => allowed.extend(["compilerCallableId"]),
            "CLASS" => allowed.extend(["compilerClassId"]),
            _ => return Err(invalid("unknown partial declaration descriptor kind")),
        }
        let object = value
            .as_object()
            .ok_or_else(|| invalid("partial declaration descriptor row is not an object"))?;
        if object.keys().any(|field| !allowed.contains(field.as_str())) {
            return Err(invalid(
                "partial declaration descriptor has an unexpected or attributed field",
            ));
        }
        match kind {
            "FUNCTION" => {
                let callable = required_string(value, "compilerCallableId")?;
                let identity = required_string(value, "symbolIdentity")?;
                if !identity.starts_with(&format!("callable:{callable}#jvm:")) {
                    return Err(invalid(
                        "partial function descriptor compiler identity is inconsistent",
                    ));
                }
                if let Some(jvm) = value.get("jvmDescriptor").and_then(Value::as_str)
                    && identity != format!("callable:{callable}#jvm:{jvm}")
                {
                    return Err(invalid(
                        "partial function descriptor JVM identity is inconsistent",
                    ));
                }
            }
            "CONSTRUCTOR" => {
                let callable = required_string(value, "compilerCallableId")?;
                let class = required_string(value, "compilerClassId")?;
                let jvm = required_string(value, "jvmDescriptor")?;
                if required_string(value, "symbolIdentity")?
                    != format!("constructor:{callable}#jvm:{jvm}")
                    || required_string(value, "ownerIdentity")? != format!("class:{class}")
                    || !jvm.starts_with('(')
                    || !jvm.contains(')')
                {
                    return Err(invalid(
                        "partial constructor descriptor compiler/JVM identity is inconsistent",
                    ));
                }
            }
            "PROPERTY" | "MUTABLE_PROPERTY" => {
                let callable = required_string(value, "compilerCallableId")?;
                if required_string(value, "symbolIdentity")? != format!("property:{callable}") {
                    return Err(invalid(
                        "partial property descriptor compiler identity is inconsistent",
                    ));
                }
            }
            "CLASS" => {
                let class = required_string(value, "compilerClassId")?;
                if required_string(value, "symbolIdentity")? != format!("class:{class}") {
                    return Err(invalid(
                        "partial class descriptor compiler identity is inconsistent",
                    ));
                }
            }
            _ => unreachable!(),
        }
        Ok(())
    }

    let graph = facts
        .get("declarationDescriptors")
        .filter(|value| value.is_object())
        .ok_or_else(|| invalid("worker index has no declaration descriptor graph"))?;
    if graph.get("schema").and_then(Value::as_str) != Some("declaration-descriptor-graph/0.1") {
        return Err(invalid("unsupported declaration descriptor graph schema"));
    }
    let compilation = required_string(graph, "compilation")?;
    if facts.get("compilation").and_then(Value::as_str) != Some(compilation) {
        return Err(ClewError::new(
            ErrorCode::ProjectModelChanged,
            "declaration descriptor graph compilation differs from worker index snapshot",
        ));
    }
    let coverage = graph.get("coverage").and_then(Value::as_str);
    if !matches!(coverage, Some("COMPLETE_SUPPORTED_SUBSET" | "PARTIAL")) {
        return Err(invalid("unknown declaration descriptor coverage status"));
    }
    let descriptors = graph
        .get("descriptors")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("declaration descriptor graph has no descriptors"))?;
    let source_binding = declaration_source_binding(facts)?;
    let bound_sources = source_binding["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| {
            Ok((
                required_string(file, "path")?.to_owned(),
                (
                    required_string(file, "module")?.to_owned(),
                    required_string(file, "sourceSet")?.to_owned(),
                ),
            ))
        })
        .collect::<Result<BTreeMap<_, _>, ClewError>>()?;
    canonical_rows(descriptors, "rows")?;
    let mut partial_descriptors = BTreeMap::new();
    for descriptor in descriptors {
        if validate_kotlin_semantic_payload(descriptor)?
            != KotlinSemanticPayloadKind::DeclarationDescriptor
        {
            return Err(invalid(
                "declaration descriptor row has the wrong payload kind",
            ));
        }
        if descriptor.get("schema").and_then(Value::as_str) != Some("declaration-descriptor/0.1")
            || descriptor.get("resolution").and_then(Value::as_str) != Some("PROVEN")
            || descriptor.get("provider").and_then(Value::as_str) != Some("K2_FIR")
            || descriptor.get("sourceProvenance").and_then(Value::as_str)
                != Some("COMPILER_UTF16_RANGE_TO_UTF8_BYTES")
            || descriptor.get("compilerAuthority").and_then(Value::as_str)
                != Some("fir-facts-extractor/0.6")
        {
            return Err(invalid(
                "malformed or non-authoritative declaration descriptor",
            ));
        }
        safe_file(descriptor)?;
        let descriptor_file = required_string(descriptor, "file")?;
        let bound_source = bound_sources
            .get(descriptor_file)
            .ok_or_else(|| invalid("declaration descriptor file is absent from source binding"))?;
        if descriptor.get("module").and_then(Value::as_str) != Some(bound_source.0.as_str())
            || descriptor.get("sourceSet").and_then(Value::as_str) != Some(bound_source.1.as_str())
        {
            return Err(ClewError::new(
                ErrorCode::ProjectModelChanged,
                "declaration descriptor module/sourceSet differs from source binding",
            ));
        }
        let start = descriptor
            .get("start")
            .and_then(Value::as_i64)
            .unwrap_or(-1);
        let end = descriptor.get("end").and_then(Value::as_i64).unwrap_or(-1);
        if start < 0 || end < start {
            return Err(invalid("declaration descriptor has invalid source range"));
        }
        for field in ["symbolIdentity", "ownerIdentity", "module", "sourceSet"] {
            required_string(descriptor, field)?;
        }
        nonempty_string_array(descriptor, "containment")?;
        let containment = descriptor["containment"].as_array().unwrap();
        let owner = descriptor
            .get("ownerIdentity")
            .and_then(Value::as_str)
            .unwrap();
        if containment
            .last()
            .and_then(Value::as_str)
            .map_or_else(|| !owner.starts_with("package:"), |last| last != owner)
        {
            return Err(invalid(
                "declaration descriptor owner differs from compiler containment",
            ));
        }
        let declaration_kind = descriptor
            .get("declarationKind")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("declaration descriptor has no declarationKind"))?;
        let has_partial_marker = descriptor.get("attributeCoverage").is_some()
            || descriptor.get("sourceRowHash").is_some();
        if has_partial_marker {
            if coverage != Some("PARTIAL")
                || descriptor.get("attributeCoverage").and_then(Value::as_str) != Some("PARTIAL")
            {
                return Err(invalid(
                    "partial declaration descriptor requires PARTIAL row and graph coverage",
                ));
            }
            validate_partial_descriptor(descriptor, declaration_kind)?;
            let source_row_hash = required_string(descriptor, "sourceRowHash")?;
            if !is_canonical_sha256(source_row_hash) {
                return Err(invalid(
                    "partial declaration descriptor source row hash is malformed",
                ));
            }
            let retained_hash = canonical::hash(descriptor).map_err(internal)?;
            if partial_descriptors
                .insert(source_row_hash.to_owned(), (retained_hash, descriptor))
                .is_some()
            {
                return Err(invalid(
                    "partial declaration descriptors repeat a source row link",
                ));
            }
            continue;
        }
        if !matches!(
            descriptor.get("visibility").and_then(Value::as_str),
            Some("public" | "internal" | "private" | "protected")
        ) || !matches!(
            descriptor
                .get("effectiveVisibility")
                .and_then(Value::as_str),
            Some("public" | "internal" | "private-in-class" | "private-in-file" | "protected")
        ) || !matches!(
            descriptor.get("exportBoundary").and_then(Value::as_str),
            Some("PUBLIC_API" | "MODULE_API" | "PRIVATE_API")
        ) || !matches!(
            descriptor.get("modality").and_then(Value::as_str),
            Some("FINAL" | "OPEN" | "ABSTRACT" | "SEALED")
        ) {
            return Err(invalid(
                "declaration descriptor has an unknown compiler enum",
            ));
        }
        let expected_export = match descriptor
            .get("effectiveVisibility")
            .and_then(Value::as_str)
        {
            Some("public" | "protected") => "PUBLIC_API",
            Some("internal") => "MODULE_API",
            Some("private-in-class" | "private-in-file") => "PRIVATE_API",
            _ => unreachable!(),
        };
        if descriptor.get("exportBoundary").and_then(Value::as_str) != Some(expected_export) {
            return Err(invalid(
                "declaration descriptor export boundary disagrees with effective visibility",
            ));
        }
        validate_type_parameters(descriptor)?;
        validate_field_closure(descriptor, declaration_kind)?;
        if let Some(spring) = descriptor.get("spring") {
            crate::spring_entrypoints::validate_metadata(spring, "K2_RESOLVED_ANNOTATIONS")?;
        }
        match declaration_kind {
            "FUNCTION" => {
                let callable = required_string(descriptor, "compilerCallableId")?;
                let identity = required_string(descriptor, "symbolIdentity")?;
                if !identity.starts_with(&format!("callable:{callable}#jvm:"))
                    || !descriptor.get("isOverride").is_some_and(Value::is_boolean)
                {
                    return Err(invalid(
                        "function descriptor compiler identity is inconsistent",
                    ));
                }
                if let Some(jvm) = descriptor.get("jvmDescriptor").and_then(Value::as_str)
                    && identity != format!("callable:{callable}#jvm:{jvm}")
                {
                    return Err(invalid("function descriptor JVM identity is inconsistent"));
                }
                validate_typed_value(descriptor, "returnType", "returnNullable")?;
                let parameters = descriptor
                    .get("parameterTypes")
                    .and_then(Value::as_array)
                    .ok_or_else(|| invalid("function descriptor has no parameterTypes"))?;
                for (index, parameter) in parameters.iter().enumerate() {
                    validate_parameter_slot(parameter, index)?;
                }
                if let Some(receiver) = descriptor.get("receiverType") {
                    validate_typed_value(receiver, "type", "nullable")?;
                }
            }
            "CONSTRUCTOR" => {
                let callable = required_string(descriptor, "compilerCallableId")?;
                let class = required_string(descriptor, "compilerClassId")?;
                let jvm = required_string(descriptor, "jvmDescriptor")?;
                if required_string(descriptor, "symbolIdentity")?
                    != format!("constructor:{callable}#jvm:{jvm}")
                    || required_string(descriptor, "ownerIdentity")? != format!("class:{class}")
                    || !descriptor.get("isPrimary").is_some_and(Value::is_boolean)
                    || !jvm.starts_with('(')
                    || !jvm.contains(')')
                {
                    return Err(invalid(
                        "constructor descriptor compiler/JVM identity is inconsistent",
                    ));
                }
                let parameters = descriptor
                    .get("parameterTypes")
                    .and_then(Value::as_array)
                    .ok_or_else(|| invalid("constructor descriptor has no parameterTypes"))?;
                let mut indices = BTreeSet::new();
                for (index, parameter) in parameters.iter().enumerate() {
                    let actual = parameter
                        .get("index")
                        .and_then(Value::as_u64)
                        .ok_or_else(|| invalid("constructor parameter has no index"))?;
                    if actual != index as u64 || !indices.insert(actual) {
                        return Err(invalid(
                            "constructor parameter indexes are not canonical and unique",
                        ));
                    }
                    validate_parameter_slot(parameter, index)?;
                }
            }
            "PROPERTY" | "MUTABLE_PROPERTY" => {
                let callable = required_string(descriptor, "compilerCallableId")?;
                if required_string(descriptor, "symbolIdentity")? != format!("property:{callable}")
                    || !descriptor.get("isOverride").is_some_and(Value::is_boolean)
                {
                    return Err(invalid(
                        "property descriptor compiler identity is inconsistent",
                    ));
                }
                validate_typed_value(descriptor, "declaredType", "declaredNullable")?;
            }
            "CLASS" => {
                let class = required_string(descriptor, "compilerClassId")?;
                if required_string(descriptor, "symbolIdentity")? != format!("class:{class}") {
                    return Err(invalid(
                        "class descriptor compiler identity is inconsistent",
                    ));
                }
            }
            _ => return Err(invalid("unknown declaration descriptor kind")),
        }
    }

    let boundaries = graph
        .get("boundaries")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("declaration descriptor graph has no boundaries"))?;
    canonical_rows(boundaries, "boundaries")?;
    let mut paired_partial_descriptors = BTreeSet::new();
    for boundary in boundaries {
        if validate_kotlin_semantic_payload(boundary)?
            != KotlinSemanticPayloadKind::DeclarationDescriptorBoundary
        {
            return Err(invalid(
                "declaration descriptor boundary has the wrong payload kind",
            ));
        }
        if boundary.get("schema").and_then(Value::as_str)
            != Some("declaration-descriptor-boundary/0.1")
            || boundary.get("resolution").and_then(Value::as_str) != Some("UNKNOWN")
            || !matches!(
                boundary.get("provider").and_then(Value::as_str),
                Some(
                    "K2_FIR"
                        | "COMPILER_DESCRIPTOR_NORMALIZER"
                        | "CODECLEW_DESCRIPTOR_NORMALIZER"
                        | "WORKER"
                )
            )
            || !matches!(
                boundary.get("stage").and_then(Value::as_str),
                Some("DECLARATION" | "CONSTRUCTOR_DECLARATION" | "NORMALIZE" | "ANALYSIS")
            )
            || !matches!(
                boundary.get("code").and_then(Value::as_str),
                Some(
                    "GENERATED_OR_NO_SOURCE"
                        | "LOCAL_DECLARATION_UNSUPPORTED"
                        | "LOCAL_GENERATED_OR_NO_SOURCE"
                        | "UNRESOLVED_DESCRIPTOR_BOUNDARY"
                        | "NO_COMPILER_CALLABLE_ID"
                        | "LOCAL_CONSTRUCTOR_UNSUPPORTED"
                        | "UNRESOLVED_CONSTRUCTOR_DESCRIPTOR"
                        | "INCOMPLETE_COMPILER_DESCRIPTOR"
                        | "MALFORMED_COMPILER_FACT_ROW"
                        | "INVALID_DESCRIPTOR_SOURCE_PATH"
                        | "INVALID_DESCRIPTOR_IDENTITY"
                        | "INVALID_JVM_DESCRIPTOR"
                        | "UNKNOWN_DECLARATION_KIND"
                        | "UNKNOWN_VISIBILITY"
                        | "UNKNOWN_EFFECTIVE_VISIBILITY"
                        | "UNKNOWN_MODALITY"
                        | "DESCRIPTOR_SOURCE_NOT_IN_COMPILATION"
                        | "INVALID_DESCRIPTOR_SOURCE_RANGE"
                        | "UNRESOLVED_DESCRIPTOR_TYPE"
                        | "JVM_NAME_OVERRIDE_UNSUPPORTED"
                        | "SYNTAX_ONLY"
                )
            )
            || required_string(boundary, "module").is_err()
            || required_string(boundary, "sourceSet").is_err()
            || boundary.get("compilerAuthority").and_then(Value::as_str)
                != Some("fir-facts-extractor/0.6")
        {
            return Err(invalid("malformed declaration descriptor Unknown boundary"));
        }
        let compiler_normalized = boundary.get("provider").and_then(Value::as_str)
            == Some("COMPILER_DESCRIPTOR_NORMALIZER");
        if boundary.get("retainedDescriptorHash").is_some() && !compiler_normalized {
            return Err(invalid(
                "retained descriptor link is outside the partial-core contract",
            ));
        }
        if compiler_normalized {
            let row_hash = required_string(boundary, "rawRowHash")?;
            if !is_canonical_sha256(row_hash) {
                return Err(invalid("descriptor boundary raw row hash is malformed"));
            }
            let optional_attribute_code = matches!(
                boundary.get("code").and_then(Value::as_str),
                Some(
                    "UNKNOWN_VISIBILITY"
                        | "UNKNOWN_EFFECTIVE_VISIBILITY"
                        | "UNKNOWN_MODALITY"
                        | "UNRESOLVED_DESCRIPTOR_TYPE"
                )
            );
            let retained_hash = boundary.get("retainedDescriptorHash");
            if optional_attribute_code && retained_hash.is_none() {
                return Err(invalid(
                    "optional descriptor attribute boundary has no retained core link",
                ));
            }
            if let Some(retained_hash) = retained_hash {
                let retained_hash = retained_hash.as_str().ok_or_else(|| {
                    invalid("descriptor boundary retained descriptor hash is not a string")
                })?;
                if !optional_attribute_code
                    || boundary.get("stage").and_then(Value::as_str) != Some("NORMALIZE")
                    || !is_canonical_sha256(retained_hash)
                {
                    return Err(invalid(
                        "retained descriptor link is outside the partial-core contract",
                    ));
                }
                let Some((expected_retained_hash, descriptor)) = partial_descriptors.get(row_hash)
                else {
                    return Err(invalid(
                        "retained descriptor boundary has no matching partial descriptor",
                    ));
                };
                if retained_hash != expected_retained_hash
                    || boundary.get("file") != descriptor.get("file")
                    || boundary.get("symbolIdentity") != descriptor.get("symbolIdentity")
                {
                    return Err(invalid(
                        "retained descriptor boundary disagrees with its exact partial descriptor",
                    ));
                }
                if !paired_partial_descriptors.insert(row_hash.to_owned()) {
                    return Err(invalid(
                        "partial declaration descriptor has duplicate retained boundary links",
                    ));
                }
            }
        }
        if boundary.get("file").is_some() {
            safe_file(boundary)?;
            let file = required_string(boundary, "file")?;
            let bound_source = bound_sources.get(file).ok_or_else(|| {
                invalid("declaration descriptor boundary file is absent from source binding")
            })?;
            if boundary.get("module").and_then(Value::as_str) != Some(bound_source.0.as_str())
                || boundary.get("sourceSet").and_then(Value::as_str)
                    != Some(bound_source.1.as_str())
            {
                return Err(ClewError::new(
                    ErrorCode::ProjectModelChanged,
                    "declaration descriptor boundary module/sourceSet differs from source binding",
                ));
            }
        }
    }
    if paired_partial_descriptors.len() != partial_descriptors.len() {
        return Err(invalid(
            "partial declaration descriptor has no matching retained boundary link",
        ));
    }

    let provenance = graph
        .get("provenance")
        .filter(|value| value.is_object())
        .ok_or_else(|| invalid("declaration descriptor graph has no authority provenance"))?;
    if provenance.get("provider").and_then(Value::as_str) != Some("COMPILER_SEMANTIC_FACTS")
        || provenance.get("extractorSchema").and_then(Value::as_str)
            != Some("fir-facts-extractor/0.6")
        || required_string(provenance, "workerVersion").is_err()
        || required_string(provenance, "workerProtocolVersion").is_err()
    {
        return Err(invalid(
            "declaration descriptor authority provenance is incomplete",
        ));
    }
    let plugin = required_string(provenance, "pluginArtifactFingerprint")?;
    if !plugin.starts_with("sha256:") || plugin.len() != 71 {
        return Err(invalid(
            "declaration descriptor plugin fingerprint is malformed",
        ));
    }
    for (provenance_field, index_field) in [
        ("compilerVersion", "compilerVersion"),
        ("workerCompilerVersion", "compilerVersion"),
        ("projectModelHash", "projectModelHash"),
        ("classpathHash", "classpathHash"),
        ("compilerOptionsHash", "compilerOptionsHash"),
    ] {
        if required_string(provenance, provenance_field)?
            != facts
                .get(index_field)
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| invalid(format!("worker index has no {index_field}")))?
        {
            return Err(ClewError::new(
                ErrorCode::ProjectModelChanged,
                format!(
                    "declaration descriptor {provenance_field} differs from worker index snapshot"
                ),
            ));
        }
    }
    let hash = required_string(facts, "declarationDescriptorHash")?.to_owned();
    if hash != canonical::hash(graph).map_err(internal)? {
        return Err(ClewError::new(
            ErrorCode::ProjectModelChanged,
            "declaration descriptor hash differs from canonical Rust graph hash",
        ));
    }
    Ok(DeclarationDescriptorSnapshot {
        graph: graph.clone(),
        hash,
        provenance: provenance.clone(),
    })
}

pub(crate) fn descriptor_validation_diagnostic(facts: &Value) -> Value {
    fn shape(value: Option<&Value>) -> Value {
        let json_type = match value {
            None => "MISSING",
            Some(Value::Null) => "NULL",
            Some(Value::Bool(_)) => "BOOLEAN",
            Some(Value::Number(_)) => "NUMBER",
            Some(Value::String(_)) => "STRING",
            Some(Value::Array(_)) => "ARRAY",
            Some(Value::Object(_)) => "OBJECT",
        };
        serde_json::json!({
            "present":value.is_some(),
            "jsonType":json_type,
            "arrayLength":value.and_then(Value::as_array).map(Vec::len),
            "valueHash":canonical::hash(value.unwrap_or(&Value::Null)).unwrap_or_else(|_| "unavailable".into()),
        })
    }
    fn typed(value: &Value, type_field: &str, nullable_field: &str) -> bool {
        let Some(rendered) = value.get(type_field).and_then(Value::as_str) else {
            return false;
        };
        let Some(nullable) = value.get(nullable_field).and_then(Value::as_bool) else {
            return false;
        };
        !rendered.contains("..")
            && !rendered.contains('!')
            && !rendered.contains("<ERROR")
            && nullable == rendered.trim_end().ends_with('?')
    }
    fn report(stage: &str, ordinal: usize, row: &Value) -> Value {
        let fields = [
            "schema",
            "resolution",
            "provider",
            "compilerAuthority",
            "declarationKind",
            "symbolIdentity",
            "ownerIdentity",
            "containment",
            "visibility",
            "effectiveVisibility",
            "exportBoundary",
            "modality",
            "compilerCallableId",
            "compilerClassId",
            "jvmDescriptor",
            "typeParameters",
            "parameterTypes",
            "returnType",
            "returnNullable",
            "declaredType",
            "declaredNullable",
            "attributeCoverage",
            "sourceRowHash",
        ];
        let shapes = fields
            .into_iter()
            .map(|field| (field.to_owned(), shape(row.get(field))))
            .collect::<serde_json::Map<_, _>>();
        let jvm_signature = match row.get("declarationKind").and_then(Value::as_str) {
            Some("FUNCTION") => row
                .get("symbolIdentity")
                .and_then(Value::as_str)
                .and_then(|identity| identity.split_once("#jvm:").map(|(_, suffix)| suffix)),
            Some("CONSTRUCTOR") => row.get("jvmDescriptor").and_then(Value::as_str),
            _ => None,
        };
        serde_json::json!({
            "schema":"descriptor-validation-diagnostic/0.1",
            "stage":stage,
            "ordinal":ordinal,
            "rowHash":canonical::hash(row).unwrap_or_else(|_| "unavailable".into()),
            "kind":row.get("declarationKind").and_then(Value::as_str).filter(|kind| matches!(*kind, "FUNCTION"|"CONSTRUCTOR"|"PROPERTY"|"MUTABLE_PROPERTY"|"CLASS")),
            "partial":row.get("attributeCoverage").and_then(Value::as_str) == Some("PARTIAL")
                || row.get("sourceRowHash").is_some(),
            "jvmShape":{
                "present":jvm_signature.is_some(),
                "byteLength":jvm_signature.map(str::len),
                "containsDot":jvm_signature.is_some_and(|value| value.contains('.')),
                "containsControl":jvm_signature.is_some_and(|value| value.chars().any(char::is_control)),
                "containsNestedClassMarker":jvm_signature.is_some_and(|value| value.contains('$')),
            },
            "shapes":shapes,
        })
    }
    let descriptors = facts
        .pointer("/declarationDescriptors/descriptors")
        .and_then(Value::as_array);
    let Some(descriptors) = descriptors else {
        return report("DESCRIPTOR_KIND_IDENTITY", 0, &Value::Null);
    };
    for (ordinal, descriptor) in descriptors.iter().enumerate() {
        let kind = descriptor.get("declarationKind").and_then(Value::as_str);
        if descriptor.get("schema").and_then(Value::as_str) != Some("declaration-descriptor/0.1")
            || descriptor.get("resolution").and_then(Value::as_str) != Some("PROVEN")
            || descriptor.get("provider").and_then(Value::as_str) != Some("K2_FIR")
            || descriptor.get("sourceProvenance").and_then(Value::as_str)
                != Some("COMPILER_UTF16_RANGE_TO_UTF8_BYTES")
            || descriptor.get("compilerAuthority").and_then(Value::as_str)
                != Some("fir-facts-extractor/0.6")
            || !matches!(
                kind,
                Some("FUNCTION" | "CONSTRUCTOR" | "PROPERTY" | "MUTABLE_PROPERTY" | "CLASS")
            )
        {
            return report("DESCRIPTOR_KIND_IDENTITY", ordinal, descriptor);
        }
        let owner = descriptor.get("ownerIdentity").and_then(Value::as_str);
        let containment = descriptor.get("containment").and_then(Value::as_array);
        if owner.is_none()
            || containment.is_none()
            || containment.is_some_and(|values| {
                values.last().and_then(Value::as_str).map_or_else(
                    || !owner.is_some_and(|value| value.starts_with("package:")),
                    |last| Some(last) != owner,
                )
            })
        {
            return report("OWNER_CONTAINMENT", ordinal, descriptor);
        }
        let partial = descriptor.get("attributeCoverage").is_some()
            || descriptor.get("sourceRowHash").is_some();
        if partial {
            if validate_declaration_descriptor_fact(descriptor).is_err() {
                return report("PARTIAL_CORE", ordinal, descriptor);
            }
            continue;
        }
        if validate_declaration_descriptor_fact(descriptor).is_err() {
            return report("EXACT_PAYLOAD", ordinal, descriptor);
        }
        let effective = descriptor
            .get("effectiveVisibility")
            .and_then(Value::as_str);
        let expected_export = match effective {
            Some("public" | "protected") => Some("PUBLIC_API"),
            Some("internal") => Some("MODULE_API"),
            Some("private-in-class" | "private-in-file") => Some("PRIVATE_API"),
            _ => None,
        };
        if !matches!(
            descriptor.get("visibility").and_then(Value::as_str),
            Some("public" | "internal" | "private" | "protected")
        ) || expected_export.is_none()
            || descriptor.get("exportBoundary").and_then(Value::as_str) != expected_export
            || !matches!(
                descriptor.get("modality").and_then(Value::as_str),
                Some("FINAL" | "OPEN" | "ABSTRACT" | "SEALED")
            )
        {
            return report("VISIBILITY_MODALITY", ordinal, descriptor);
        }
        let jvm_valid = match kind.unwrap() {
            "FUNCTION" => match (
                descriptor.get("compilerCallableId").and_then(Value::as_str),
                descriptor.get("symbolIdentity").and_then(Value::as_str),
            ) {
                (Some(callable), Some(identity)) => {
                    identity.starts_with(&format!("callable:{callable}#jvm:"))
                }
                _ => false,
            },
            "CONSTRUCTOR" => match (
                descriptor.get("compilerCallableId").and_then(Value::as_str),
                descriptor.get("compilerClassId").and_then(Value::as_str),
                descriptor.get("jvmDescriptor").and_then(Value::as_str),
                descriptor.get("symbolIdentity").and_then(Value::as_str),
                owner,
            ) {
                (Some(callable), Some(class), Some(jvm), Some(identity), Some(owner)) => {
                    identity == format!("constructor:{callable}#jvm:{jvm}")
                        && owner == format!("class:{class}")
                        && jvm.starts_with('(')
                        && jvm.contains(')')
                }
                _ => false,
            },
            "PROPERTY" | "MUTABLE_PROPERTY" => match (
                descriptor.get("compilerCallableId").and_then(Value::as_str),
                descriptor.get("symbolIdentity").and_then(Value::as_str),
            ) {
                (Some(callable), Some(identity)) => identity == format!("property:{callable}"),
                _ => false,
            },
            "CLASS" => match (
                descriptor.get("compilerClassId").and_then(Value::as_str),
                descriptor.get("symbolIdentity").and_then(Value::as_str),
            ) {
                (Some(class), Some(identity)) => identity == format!("class:{class}"),
                _ => false,
            },
            _ => false,
        };
        if !jvm_valid {
            return report("JVM_SIGNATURE", ordinal, descriptor);
        }
        if matches!(kind, Some("FUNCTION" | "CONSTRUCTOR")) {
            let Some(parameters) = descriptor.get("parameterTypes").and_then(Value::as_array)
            else {
                return report("PARAMETER_SLOTS", ordinal, descriptor);
            };
            if parameters.iter().enumerate().any(|(index, parameter)| {
                parameter.get("index").and_then(Value::as_u64) != Some(index as u64)
            }) {
                return report("PARAMETER_SLOTS", ordinal, descriptor);
            }
            if parameters
                .iter()
                .any(|parameter| !typed(parameter, "type", "nullable"))
            {
                return report("TYPE_NULLABILITY", ordinal, descriptor);
            }
        }
        if kind == Some("FUNCTION") && !typed(descriptor, "returnType", "returnNullable") {
            return report("TYPE_NULLABILITY", ordinal, descriptor);
        }
        if matches!(kind, Some("PROPERTY" | "MUTABLE_PROPERTY"))
            && !typed(descriptor, "declaredType", "declaredNullable")
        {
            return report("TYPE_NULLABILITY", ordinal, descriptor);
        }
    }
    if let Some(boundaries) = facts
        .pointer("/declarationDescriptors/boundaries")
        .and_then(Value::as_array)
    {
        for (ordinal, boundary) in boundaries.iter().enumerate() {
            if boundary.get("schema").and_then(Value::as_str)
                != Some("declaration-descriptor-boundary/0.1")
                || boundary.get("resolution").and_then(Value::as_str) != Some("UNKNOWN")
            {
                return report("UNKNOWN_BOUNDARY", ordinal, boundary);
            }
        }
    }
    report("DESCRIPTOR_KIND_IDENTITY", descriptors.len(), &Value::Null)
}

fn internal(error: anyhow::Error) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn verified_facts() -> Value {
        let relation_graph = json!({
            "schema":"declaration-relation-graph/0.1",
            "compilation":":/main",
            "coverage":"PARTIAL",
            "relations":[{
                "schema":"declaration-relation/0.1",
                "file":"A.kt","start":0,"end":12,
                "kind":"OVERRIDES","owner":"p/Derived.read","target":"p/Base.read",
                "resolution":"PROVEN","provider":"K2_FIR","cfgNodeIds":[],
                "sourceProvenance":"COMPILER_UTF16_RANGE_TO_UTF8_BYTES",
                "orderProvenance":"UNKNOWN"
            }],
            "boundaries":[{
                "schema":"declaration-relation-boundary/0.1",
                "file":"A.kt","start":0,"end":12,"owner":"p/Derived.label",
                "stage":"OVERRIDE","code":"NON_FUNCTION_OVERRIDE_UNSUPPORTED",
                "resolution":"UNKNOWN","provider":"K2_FIR"
            }],
            "provenance":provenance('a')
        });
        let descriptor_graph = json!({
            "schema":"declaration-descriptor-graph/0.1",
            "compilation":":/main",
            "coverage":"PARTIAL",
            "descriptors":[{
                "schema":"declaration-descriptor/0.1",
                "file":"A.kt","start":0,"end":12,
                "symbolIdentity":"callable:p/Derived.read#jvm:()I",
                "declarationKind":"FUNCTION","ownerIdentity":"class:p/Derived",
                "containment":["class:p/Derived"],"visibility":"public",
                "effectiveVisibility":"public","exportBoundary":"PUBLIC_API",
                "modality":"FINAL","compilerCallableId":"p/Derived.read","isOverride":true,
                "returnType":"kotlin/Int","returnNullable":false,
                "parameterTypes":[],"typeParameters":[],"module":":","sourceSet":"main",
                "sourceProvenance":"COMPILER_UTF16_RANGE_TO_UTF8_BYTES",
                "compilerAuthority":"fir-facts-extractor/0.6",
                "resolution":"PROVEN","provider":"K2_FIR"
            }],
            "boundaries":[{
                "schema":"declaration-descriptor-boundary/0.1",
                "file":"A.kt","start":4,"end":8,"stage":"DECLARATION",
                "code":"NO_COMPILER_CALLABLE_ID","resolution":"UNKNOWN","provider":"K2_FIR",
                "module":":","sourceSet":"main","compilerAuthority":"fir-facts-extractor/0.6"
            }],
            "provenance":provenance('b')
        });
        json!({
            "compilation":":/main","partial":false,
            "projectModelHash":"model","classpathHash":"classpath",
            "compilerVersion":"2.4.10","compilerOptionsHash":"options",
            "declarationRelationHash":canonical::hash(&relation_graph).unwrap(),
            "declarationRelations":relation_graph,
            "declarationDescriptorHash":canonical::hash(&descriptor_graph).unwrap(),
            "declarationDescriptors":descriptor_graph,
            "files":[{
                "path":"A.kt","module":":","sourceSet":"main",
                "contentHash":canonical::hash_bytes(b"fun read() = 1\n"),
                "declarations":[{"symbolId":"p/Derived.read"}],"semanticFacts":[]
            }]
        })
    }

    fn provenance(fingerprint: char) -> Value {
        json!({
            "provider":"COMPILER_SEMANTIC_FACTS",
            "extractorSchema":"fir-facts-extractor/0.6",
            "pluginArtifactFingerprint":format!("sha256:{}", fingerprint.to_string().repeat(64)),
            "workerCompilerVersion":"2.4.10","workerVersion":"0.1.0",
            "workerProtocolVersion":"1.0","compilerVersion":"2.4.10",
            "projectModelHash":"model","classpathHash":"classpath",
            "compilerOptionsHash":"options"
        })
    }

    fn refresh(facts: &mut Value, graph: &str, hash: &str) {
        facts[hash] = Value::String(canonical::hash(&facts[graph]).unwrap());
    }

    fn constructor_fact() -> Value {
        json!({
            "schema":"declaration-descriptor/0.1",
            "file":"A.kt","start":0,"end":12,
            "symbolIdentity":"constructor:p/Box.<init>#jvm:(I[Ljava/lang/String;)V",
            "declarationKind":"CONSTRUCTOR","ownerIdentity":"class:p/Box",
            "containment":["class:p/Box"],"visibility":"public",
            "effectiveVisibility":"public","exportBoundary":"PUBLIC_API",
            "modality":"FINAL","compilerCallableId":"p/Box.<init>",
            "compilerClassId":"p/Box","isPrimary":true,
            "jvmDescriptor":"(I[Ljava/lang/String;)V",
            "parameterTypes":[
                {"index":0,"type":"kotlin/Int","nullable":false},
                {"index":1,"type":"kotlin/Array<kotlin/String>","nullable":false}
            ],
            "typeParameters":[],"module":":","sourceSet":"main",
            "sourceProvenance":"COMPILER_UTF16_RANGE_TO_UTF8_BYTES",
            "compilerAuthority":"fir-facts-extractor/0.6",
            "resolution":"PROVEN","provider":"K2_FIR"
        })
    }

    #[test]
    fn strict_jvm_method_descriptor_parser_accepts_the_supported_field_grammar() {
        for descriptor in [
            "()V",
            "(BCDFIJSZ)I",
            "(Ljava/lang/String;)Ljava/lang/Object;",
            "([[I[[[Ljava/util/Map$Entry;)[Ljava/lang/String;",
            "(Lkotlin/Pair;)Z",
        ] {
            validate_jvm_method_descriptor(descriptor).unwrap();
        }
        let maximum_array = format!("({}I)V", "[".repeat(255));
        validate_jvm_method_descriptor(&maximum_array).unwrap();
        validate_jvm_method_descriptor(&format!("({})V", "I".repeat(255))).unwrap();
        validate_jvm_method_descriptor(&format!("({}I)V", "J".repeat(127))).unwrap();
        validate_jvm_method_descriptor(&format!("({})V", "[J".repeat(255))).unwrap();
    }

    #[test]
    fn strict_jvm_method_descriptor_parser_rejects_malformed_and_trailing_shapes() {
        for descriptor in [
            "",
            "I",
            "(",
            "()",
            "()VV",
            "(V)V",
            "([V)V",
            "([)V",
            "(Q)V",
            "(L;)V",
            "(Ljava//lang/String;)V",
            "(Ljava/lang/String)V",
            "(Ljava.lang.String;)V",
            "()L;",
            "()[Igarbage",
            "()Ljava/lang/String;;",
        ] {
            assert!(
                validate_jvm_method_descriptor(descriptor).is_err(),
                "accepted malformed descriptor {descriptor:?}"
            );
        }
        let oversized_array = format!("({}I)V", "[".repeat(256));
        assert!(validate_jvm_method_descriptor(&oversized_array).is_err());
        assert!(validate_jvm_method_descriptor(&format!("({})V", "I".repeat(256))).is_err());
        assert!(validate_jvm_method_descriptor(&format!("({})V", "J".repeat(128))).is_err());
        assert!(validate_jvm_method_descriptor(&format!("({})V", "[J".repeat(256))).is_err());
    }

    #[test]
    fn full_symbol_validator_rejects_heuristic_and_privacy_lookalikes() {
        for identity in [
            "callable:p/Api.read#jvm:()I",
            "callable:p/Api.read#jvm:read()I",
            "constructor:p/Box.<init>#jvm:(I)V",
            "property:p/Box.value",
            "callable:/toLocalDateTime#jvm:(Ljava/lang/String;)Ljava/time/LocalDateTime;",
            "property:/rootFlag",
            "class:p/Box",
        ] {
            validate_kotlin_full_symbol_identity(identity).unwrap();
        }
        for identity in [
            "callable:p/Api.read#jvm:bad",
            "constructor:p/Box.<init>#jvm:(I)I",
            "package:p",
            concat!("callable:https://user:", "secret", "@example/x#jvm:()I"),
            concat!("class:git", "@example.com:private/repository"),
            "class:/Users",
            concat!("callable:/", "Users/alice#jvm:()V"),
            concat!("class:/", "Users/alice/private"),
            concat!("callable:/", "Users/alice/Foo.read#jvm:()I"),
            "property:..\\private",
        ] {
            assert!(
                validate_kotlin_full_symbol_identity(identity).is_err(),
                "accepted unsafe or malformed full identity {identity:?}"
            );
        }
    }

    #[test]
    fn granular_descriptor_validation_accepts_legacy_and_current_k2_jvm_shapes() {
        let function = verified_facts()["declarationDescriptors"]["descriptors"][0].clone();
        assert_eq!(
            validate_kotlin_semantic_payload(&function).unwrap(),
            KotlinSemanticPayloadKind::DeclarationDescriptor
        );

        let mut named_signature = function.clone();
        named_signature["symbolIdentity"] = json!("callable:p/Derived.read#jvm:read()I");
        validate_declaration_descriptor_fact(&named_signature).unwrap();
        validate_declaration_descriptor_fact(&constructor_fact()).unwrap();

        let mut partial = named_signature;
        let object = partial.as_object_mut().unwrap();
        for field in [
            "visibility",
            "effectiveVisibility",
            "exportBoundary",
            "modality",
            "isOverride",
            "returnType",
            "returnNullable",
            "parameterTypes",
            "typeParameters",
        ] {
            object.remove(field);
        }
        object.insert("attributeCoverage".into(), json!("PARTIAL"));
        object.insert(
            "sourceRowHash".into(),
            json!(format!("sha256:{}", "a".repeat(64))),
        );
        validate_declaration_descriptor_fact(&partial).unwrap();
    }

    #[test]
    fn descriptor_line_coordinates_are_optional_but_closed_when_present() {
        let legacy = verified_facts()["declarationDescriptors"]["descriptors"][0].clone();
        validate_declaration_descriptor_fact(&legacy).unwrap();

        let mut located = legacy.clone();
        located["startLine"] = json!(4);
        located["endLine"] = json!(9);
        located["lineProvenance"] = json!("UTF8_BYTE_RANGE_OVER_COMPILATION_SOURCE");
        validate_declaration_descriptor_fact(&located).unwrap();

        let mut missing = located.clone();
        missing.as_object_mut().unwrap().remove("endLine");
        assert!(validate_declaration_descriptor_fact(&missing).is_err());

        for (field, value) in [
            ("startLine", json!(0)),
            ("endLine", json!(3)),
            ("lineProvenance", json!("FIR_GUESSED_LINES")),
        ] {
            let mut invalid = located.clone();
            invalid[field] = value;
            assert!(validate_declaration_descriptor_fact(&invalid).is_err());
        }
    }

    #[test]
    fn partial_descriptor_admission_keeps_jvm_suffix_opaque_but_safe() {
        let source_row_hash = format!("sha256:{}", "a".repeat(64));

        let mut partial_function =
            verified_facts()["declarationDescriptors"]["descriptors"][0].clone();
        partial_function["symbolIdentity"] =
            json!("callable:p/Derived.read#jvm:(Lcompiler.rendered.Type;)I");
        let function = partial_function.as_object_mut().unwrap();
        for field in [
            "visibility",
            "effectiveVisibility",
            "exportBoundary",
            "modality",
            "isOverride",
            "returnType",
            "returnNullable",
            "parameterTypes",
            "typeParameters",
        ] {
            function.remove(field);
        }
        function.insert("attributeCoverage".into(), json!("PARTIAL"));
        function.insert("sourceRowHash".into(), json!(source_row_hash));
        validate_declaration_descriptor_fact(&partial_function).unwrap();

        let mut partial_constructor = constructor_fact();
        partial_constructor["jvmDescriptor"] = json!("(Lcompiler.rendered.Type;)");
        partial_constructor["symbolIdentity"] =
            json!("constructor:p/Box.<init>#jvm:(Lcompiler.rendered.Type;)");
        let constructor = partial_constructor.as_object_mut().unwrap();
        for field in [
            "visibility",
            "effectiveVisibility",
            "exportBoundary",
            "modality",
            "isPrimary",
            "parameterTypes",
            "typeParameters",
        ] {
            constructor.remove(field);
        }
        constructor.insert("attributeCoverage".into(), json!("PARTIAL"));
        constructor.insert(
            "sourceRowHash".into(),
            json!(format!("sha256:{}", "b".repeat(64))),
        );
        validate_declaration_descriptor_fact(&partial_constructor).unwrap();

        for unsafe_suffix in [
            "",
            "()V#jvm:forged",
            "https://example.invalid()V",
            "bad@id()V",
        ] {
            let mut unsafe_partial = partial_function.clone();
            unsafe_partial["symbolIdentity"] =
                json!(format!("callable:p/Derived.read#jvm:{unsafe_suffix}"));
            assert!(validate_declaration_descriptor_fact(&unsafe_partial).is_err());
        }
    }

    #[test]
    fn partial_descriptor_snapshot_requires_exact_unknown_boundary_pair() {
        let mut facts = verified_facts();
        let source_row_hash = format!("sha256:{}", "c".repeat(64));
        let descriptor = facts["declarationDescriptors"]["descriptors"][0]
            .as_object_mut()
            .unwrap();
        descriptor.insert(
            "symbolIdentity".into(),
            json!("callable:p/Derived.read#jvm:(Lcompiler.rendered.Type;)I"),
        );
        for field in [
            "visibility",
            "effectiveVisibility",
            "exportBoundary",
            "modality",
            "isOverride",
            "returnType",
            "returnNullable",
            "parameterTypes",
            "typeParameters",
        ] {
            descriptor.remove(field);
        }
        descriptor.insert("attributeCoverage".into(), json!("PARTIAL"));
        descriptor.insert("sourceRowHash".into(), json!(source_row_hash));
        let retained_hash = canonical::hash(&Value::Object(descriptor.clone())).unwrap();
        facts["declarationDescriptors"]["boundaries"] = json!([{
            "schema":"declaration-descriptor-boundary/0.1",
            "file":"A.kt","start":0,"end":12,
            "symbolIdentity":"callable:p/Derived.read#jvm:(Lcompiler.rendered.Type;)I",
            "stage":"NORMALIZE","code":"UNKNOWN_VISIBILITY",
            "resolution":"UNKNOWN","provider":"COMPILER_DESCRIPTOR_NORMALIZER",
            "module":":","sourceSet":"main",
            "sourceProvenance":"COMPILER_UTF16_RANGE_TO_UTF8_BYTES",
            "compilerAuthority":"fir-facts-extractor/0.6",
            "rawRowHash":format!("sha256:{}", "c".repeat(64)),
            "retainedDescriptorHash":retained_hash,
        }]);
        refresh(
            &mut facts,
            "declarationDescriptors",
            "declarationDescriptorHash",
        );

        validate_declaration_descriptor_snapshot(&facts).unwrap();

        facts["declarationDescriptors"]["boundaries"][0]["retainedDescriptorHash"] =
            json!(format!("sha256:{}", "d".repeat(64)));
        refresh(
            &mut facts,
            "declarationDescriptors",
            "declarationDescriptorHash",
        );
        assert!(validate_declaration_descriptor_snapshot(&facts).is_err());
    }

    #[test]
    fn descriptor_snapshot_accepts_k2_jvm_name_override_boundary() {
        let mut facts = verified_facts();
        facts["declarationDescriptors"]["boundaries"][0]["code"] =
            json!("JVM_NAME_OVERRIDE_UNSUPPORTED");
        refresh(
            &mut facts,
            "declarationDescriptors",
            "declarationDescriptorHash",
        );

        validate_declaration_descriptor_snapshot(&facts).unwrap();
    }

    #[test]
    fn descriptor_snapshot_rejects_unknown_k2_boundary_code() {
        let mut facts = verified_facts();
        facts["declarationDescriptors"]["boundaries"][0]["code"] =
            json!("FUTURE_UNRECOGNIZED_BOUNDARY");
        refresh(
            &mut facts,
            "declarationDescriptors",
            "declarationDescriptorHash",
        );

        assert!(validate_declaration_descriptor_snapshot(&facts).is_err());
    }

    #[test]
    fn granular_descriptor_validation_rejects_bad_jvm_grammar_and_identity_substitution() {
        for identity in [
            "callable:p/Derived.read#jvm:read(I",
            "callable:p/Derived.read#jvm:read()Ix",
            "callable:p/Derived.read#jvm:read(V)I",
            "callable:p/Derived.read#jvm:read([V)I",
            "callable:p/Derived.read#jvm:(Lcompiler.rendered.Type;)I",
            "callable:p/Other.read#jvm:read()I",
        ] {
            let mut function = verified_facts()["declarationDescriptors"]["descriptors"][0].clone();
            function["symbolIdentity"] = json!(identity);
            assert!(
                validate_declaration_descriptor_fact(&function).is_err(),
                "accepted malformed function identity {identity:?}"
            );
        }

        let mut constructor = constructor_fact();
        constructor["jvmDescriptor"] = json!("(I)I");
        constructor["symbolIdentity"] = json!("constructor:p/Box.<init>#jvm:(I)I");
        assert!(validate_declaration_descriptor_fact(&constructor).is_err());

        let mut incomplete_constructor = constructor_fact();
        incomplete_constructor["jvmDescriptor"] = json!("(Lcompiler.rendered.Type;)");
        incomplete_constructor["symbolIdentity"] =
            json!("constructor:p/Box.<init>#jvm:(Lcompiler.rendered.Type;)");
        assert!(validate_declaration_descriptor_fact(&incomplete_constructor).is_err());
        assert!(has_quarantinable_exact_jvm_descriptor(
            &incomplete_constructor
        ));
        assert!(!has_quarantinable_exact_jvm_descriptor(&constructor_fact()));

        let mut incomplete_function =
            verified_facts()["declarationDescriptors"]["descriptors"][0].clone();
        incomplete_function["symbolIdentity"] =
            json!("callable:p/Derived.read#jvm:(Lcompiler.rendered.Type;)I");
        incomplete_function["jvmDescriptor"] = json!("(Lcompiler.rendered.Type;)I");
        assert!(has_quarantinable_exact_jvm_descriptor(&incomplete_function));

        let mut inconsistent_function = incomplete_function.clone();
        inconsistent_function["jvmDescriptor"] = json!("(Ljava/lang/String;)I");
        assert!(!has_quarantinable_exact_jvm_descriptor(
            &inconsistent_function
        ));

        let mut inconsistent_constructor = incomplete_constructor.clone();
        inconsistent_constructor["ownerIdentity"] = json!("class:p/Decoy");
        assert!(!has_quarantinable_exact_jvm_descriptor(
            &inconsistent_constructor
        ));

        let mut future_field = constructor_fact();
        future_field["futureAuthority"] = json!(true);
        assert!(validate_declaration_descriptor_fact(&future_field).is_err());
    }

    #[test]
    fn granular_relation_validation_closes_full_and_raw_endpoint_payloads() {
        let relation = verified_facts()["declarationRelations"]["relations"][0].clone();
        assert_eq!(
            validate_kotlin_semantic_payload(&relation).unwrap(),
            KotlinSemanticPayloadKind::DeclarationRelation
        );

        let mut full_symbols = relation.clone();
        full_symbols["owner"] = json!("callable:p/Derived.read#jvm:read()I");
        full_symbols["target"] = json!("callable:p/Base.read#jvm:()I");
        validate_declaration_relation_fact(&full_symbols).unwrap();

        let mut root_package_symbols = relation.clone();
        root_package_symbols["owner"] = json!("/rootFunction");
        root_package_symbols["target"] = json!("/rootDependency");
        validate_declaration_relation_fact(&root_package_symbols).unwrap();

        for endpoint in [
            "callable:p/Derived.read#jvm:read(V)I",
            "callable:p/Derived.read#jvm:read()Ix",
            "constructor:p/Box.<init>#jvm:(I)I",
            "p/Derived.read#jvm:()I",
        ] {
            let mut malformed = relation.clone();
            malformed["owner"] = json!(endpoint);
            assert!(
                validate_declaration_relation_fact(&malformed).is_err(),
                "accepted malformed relation endpoint {endpoint:?}"
            );
        }

        let mut unexpected = relation;
        unexpected["futureEndpoint"] = json!("p/Future.call");
        assert!(validate_declaration_relation_fact(&unexpected).is_err());
    }

    fn exact_call_relation() -> Value {
        json!({
            "schema":"declaration-relation/0.1",
            "file":"A.kt","start":10,"end":40,
            "kind":"CALLS","owner":"p/Caller.run",
            "target":"callable:p/Api.pick#jvm:(Ljava/lang/String;I)Ljava/lang/String;",
            "targetCompilerCallableId":"p/Api.pick",
            "targetJvmDescriptor":"(Ljava/lang/String;I)Ljava/lang/String;",
            "resolution":"PROVEN","provider":"K2_FIR","cfgNodeIds":[7],
            "sourceProvenance":"COMPILER_UTF16_RANGE_TO_UTF8_BYTES",
            "orderProvenance":"K2_FIR_CFG","orderKey":10,
            "resultType":"kotlin/String","receiverSelection":"EXPLICIT",
            "receiverType":"p/Api","omittedDefaultParameterIndices":[1],
            "argumentToParameter":[{
                "argumentStart":20,"argumentEnd":25,"argumentName":"value",
                "argumentType":"kotlin/String","parameter":"value",
                "parameterIndex":0,"parameterType":"kotlin/String"
            }]
        })
    }

    fn exact_call_snapshot() -> Value {
        let mut facts = verified_facts();
        facts["declarationRelations"]["relations"] = json!([exact_call_relation()]);
        facts["declarationDescriptors"]["descriptors"] = json!([{
            "schema":"declaration-descriptor/0.1",
            "file":"A.kt","start":0,"end":12,
            "symbolIdentity":"callable:p/Api.pick#jvm:(Ljava/lang/String;I)Ljava/lang/String;",
            "declarationKind":"FUNCTION","ownerIdentity":"class:p/Api",
            "containment":["class:p/Api"],"visibility":"public",
            "effectiveVisibility":"public","exportBoundary":"PUBLIC_API",
            "modality":"FINAL","compilerCallableId":"p/Api.pick",
            "jvmDescriptor":"(Ljava/lang/String;I)Ljava/lang/String;",
            "isOverride":false,"returnType":"kotlin/String","returnNullable":false,
            "parameterTypes":[
                {"index":0,"type":"kotlin/String","nullable":false,"hasDefault":false},
                {"index":1,"type":"kotlin/Int","nullable":false,"hasDefault":true}
            ],
            "typeParameters":[],"module":":","sourceSet":"main",
            "sourceProvenance":"COMPILER_UTF16_RANGE_TO_UTF8_BYTES",
            "compilerAuthority":"fir-facts-extractor/0.6",
            "resolution":"PROVEN","provider":"K2_FIR"
        }]);
        refresh(
            &mut facts,
            "declarationRelations",
            "declarationRelationHash",
        );
        refresh(
            &mut facts,
            "declarationDescriptors",
            "declarationDescriptorHash",
        );
        facts
    }

    #[test]
    fn exact_call_target_and_default_argument_mapping_are_closed_and_overload_specific() {
        let exact = exact_call_relation();
        validate_declaration_relation_fact(&exact).unwrap();

        // The compiler-selected descriptor has two slots: one mapped argument
        // and one explicitly identified default-bearing omission.
        assert_eq!(
            parse_jvm_method_descriptor(exact["targetJvmDescriptor"].as_str().unwrap())
                .unwrap()
                .parameter_count,
            2
        );

        for (field, replacement) in [
            ("target", json!("p/Api.pick")),
            ("targetCompilerCallableId", json!("p/Api.other")),
            ("targetJvmDescriptor", json!("(I)Ljava/lang/String;")),
        ] {
            let mut malformed = exact.clone();
            malformed[field] = replacement;
            assert!(
                validate_declaration_relation_fact(&malformed).is_err(),
                "accepted mismatched exact call field {field}"
            );
        }

        for missing in ["targetCompilerCallableId", "targetJvmDescriptor"] {
            let mut malformed = exact.clone();
            malformed.as_object_mut().unwrap().remove(missing);
            assert!(validate_declaration_relation_fact(&malformed).is_err());
        }

        let mut constructor = exact.clone();
        constructor["kind"] = json!("CONSTRUCTS");
        constructor["target"] = json!("constructor:p/Box.<init>#jvm:(I)I");
        constructor["targetCompilerCallableId"] = json!("p/Box.<init>");
        constructor["targetJvmDescriptor"] = json!("(I)I");
        constructor["receiverSelection"] = json!("NONE");
        constructor.as_object_mut().unwrap().remove("receiverType");
        assert!(validate_declaration_relation_fact(&constructor).is_err());

        for pointer in ["/receiverSelection", "/omittedDefaultParameterIndices"] {
            let mut missing = exact.clone();
            missing
                .as_object_mut()
                .unwrap()
                .remove(pointer.trim_start_matches('/'));
            assert!(validate_declaration_relation_fact(&missing).is_err());
        }

        let mut implicit_receiver = exact.clone();
        implicit_receiver["receiverSelection"] = json!("DISPATCH");
        assert!(validate_declaration_relation_fact(&implicit_receiver).is_err());
    }

    #[test]
    fn exact_argument_mapping_rejects_ambiguous_ranges_indices_and_open_fields() {
        let exact = exact_call_relation();
        for mutation in [
            ("/argumentToParameter/0/argumentEnd", Value::Null),
            ("/argumentToParameter/0/argumentName", json!("")),
            ("/argumentToParameter/0/parameterIndex", json!(2)),
            ("/argumentToParameter/0/argumentStart", json!(9)),
            ("/argumentToParameter/0/argumentEnd", json!(41)),
        ] {
            let mut malformed = exact.clone();
            if mutation.1.is_null() {
                malformed["argumentToParameter"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove("argumentEnd");
            } else {
                *malformed.pointer_mut(mutation.0).unwrap() = mutation.1;
            }
            assert!(validate_declaration_relation_fact(&malformed).is_err());
        }

        let mut duplicate_index = exact.clone();
        duplicate_index["argumentToParameter"] = json!([
            exact["argumentToParameter"][0].clone(),
            {
                "argumentStart":26,"argumentEnd":27,"argumentType":"kotlin/Int",
                "parameter":"value","parameterIndex":0,"parameterType":"kotlin/String"
            }
        ]);
        assert!(validate_declaration_relation_fact(&duplicate_index).is_err());

        let mut unsorted = exact.clone();
        unsorted["argumentToParameter"] = json!([
            {
                "argumentStart":26,"argumentEnd":27,"argumentType":"kotlin/Int",
                "parameter":"count","parameterIndex":1,"parameterType":"kotlin/Int"
            },
            exact["argumentToParameter"][0].clone()
        ]);
        assert!(validate_declaration_relation_fact(&unsorted).is_err());

        for omitted in [json!([]), json!([0]), json!([2]), json!([1, 1])] {
            let mut malformed = exact.clone();
            malformed["omittedDefaultParameterIndices"] = omitted;
            assert!(validate_declaration_relation_fact(&malformed).is_err());
        }

        let mut open = exact;
        open["argumentToParameter"][0]["sourceText"] = json!("private");
        assert!(validate_declaration_relation_fact(&open).is_err());
    }

    #[test]
    fn exact_call_snapshot_requires_compiler_confirmed_default_slot() {
        let facts = exact_call_snapshot();
        validate_declaration_descriptor_snapshot(&facts).unwrap();
        validate_declaration_relation_snapshot(&facts).unwrap();

        let mut no_default_authority = facts.clone();
        no_default_authority["declarationDescriptors"]["descriptors"][0]["parameterTypes"][1]["hasDefault"] =
            json!(false);
        refresh(
            &mut no_default_authority,
            "declarationDescriptors",
            "declarationDescriptorHash",
        );
        validate_declaration_descriptor_snapshot(&no_default_authority).unwrap();
        assert!(validate_declaration_relation_snapshot(&no_default_authority).is_err());

        let mut absent_default_authority = facts;
        absent_default_authority["declarationDescriptors"]["descriptors"][0]["parameterTypes"][1]
            .as_object_mut()
            .unwrap()
            .remove("hasDefault");
        refresh(
            &mut absent_default_authority,
            "declarationDescriptors",
            "declarationDescriptorHash",
        );
        assert!(validate_declaration_relation_snapshot(&absent_default_authority).is_err());
    }

    #[test]
    fn dispatcher_keeps_unknown_boundaries_distinct_from_proven_facts() {
        let facts = verified_facts();
        let descriptor_boundary = &facts["declarationDescriptors"]["boundaries"][0];
        let relation_boundary = &facts["declarationRelations"]["boundaries"][0];
        assert_eq!(
            validate_kotlin_semantic_payload(descriptor_boundary).unwrap(),
            KotlinSemanticPayloadKind::DeclarationDescriptorBoundary
        );
        assert_eq!(
            validate_kotlin_semantic_payload(relation_boundary).unwrap(),
            KotlinSemanticPayloadKind::DeclarationRelationBoundary
        );

        let mut extended = relation_boundary.clone();
        extended["futureAuthority"] = json!(true);
        assert!(validate_kotlin_semantic_payload(&extended).is_err());
        assert!(
            validate_kotlin_semantic_payload(&json!({
                "schema":"declaration-relation/0.2"
            }))
            .is_err()
        );
    }

    #[test]
    fn positional_coordinate_failure_is_a_closed_compiler_boundary() {
        let mut facts = verified_facts();
        let boundary = json!({
            "schema":"declaration-relation-boundary/0.1",
            "file":"A.kt","start":0,"end":12,
            "owner":"p/Derived.read","target":"p/Base.read",
            "relationKind":"READS","stage":"COORDINATE_NORMALIZATION",
            "code":"INVALID_RELATION_POSITIONAL_COORDINATE",
            "resolution":"UNKNOWN","provider":"COMPILER_RELATION_NORMALIZER",
            "rawRowHash":format!("sha256:{}", "c".repeat(64))
        });
        validate_declaration_relation_boundary(&boundary).unwrap();
        let mut wrong_stage = boundary.clone();
        wrong_stage["stage"] = json!("NORMALIZE");
        assert!(validate_declaration_relation_boundary(&wrong_stage).is_err());
        let boundaries = facts["declarationRelations"]["boundaries"]
            .as_array_mut()
            .unwrap();
        boundaries.push(boundary);
        boundaries.sort_by_key(|row| canonical::bytes(row).unwrap());
        refresh(
            &mut facts,
            "declarationRelations",
            "declarationRelationHash",
        );

        validate_declaration_relation_snapshot(&facts).unwrap();
    }

    #[test]
    fn sealed_graphs_round_trip_with_exact_hash_and_provenance() {
        let facts = verified_facts();
        let relation = validate_declaration_relation_snapshot(&facts).unwrap();
        let descriptor = validate_declaration_descriptor_snapshot(&facts).unwrap();
        assert_eq!(relation.graph, facts["declarationRelations"]);
        assert_eq!(relation.hash, facts["declarationRelationHash"]);
        assert_eq!(descriptor.graph, facts["declarationDescriptors"]);
        assert_eq!(descriptor.hash, facts["declarationDescriptorHash"]);
        assert_eq!(descriptor.provenance["provider"], "COMPILER_SEMANTIC_FACTS");
    }

    #[test]
    fn descriptor_shape_and_semantics_fail_closed_with_focused_diagnostic() {
        for (pointer, replacement) in [
            (
                "/declarationDescriptors/descriptors/0/exportBoundary",
                json!("PRIVATE_API"),
            ),
            (
                "/declarationDescriptors/descriptors/0/returnNullable",
                json!(true),
            ),
            (
                "/declarationDescriptors/descriptors/0/ownerIdentity",
                json!("class:p/Decoy"),
            ),
        ] {
            let mut facts = verified_facts();
            *facts.pointer_mut(pointer).unwrap() = replacement;
            refresh(
                &mut facts,
                "declarationDescriptors",
                "declarationDescriptorHash",
            );
            assert_eq!(
                validate_declaration_descriptor_snapshot(&facts)
                    .unwrap_err()
                    .code,
                ErrorCode::InvalidInput
            );
            assert_ne!(
                descriptor_validation_diagnostic(&facts)["rowHash"],
                Value::String("unavailable".into())
            );
        }
    }

    #[test]
    fn descriptor_failure_diagnostic_distinguishes_partial_rows() {
        let mut facts = verified_facts();
        let descriptor = facts["declarationDescriptors"]["descriptors"][0]
            .as_object_mut()
            .unwrap();
        for field in [
            "visibility",
            "effectiveVisibility",
            "exportBoundary",
            "modality",
            "isOverride",
            "returnType",
            "returnNullable",
            "parameterTypes",
            "typeParameters",
        ] {
            descriptor.remove(field);
        }
        descriptor.insert("attributeCoverage".into(), json!("PARTIAL"));
        descriptor.insert(
            "sourceRowHash".into(),
            json!(format!("sha256:{}", "a".repeat(64))),
        );
        descriptor.insert(
            "symbolIdentity".into(),
            json!("callable:p/Derived.read#jvm:unsafe@identity()I"),
        );

        let diagnostic = descriptor_validation_diagnostic(&facts);
        assert_eq!(diagnostic["stage"], "PARTIAL_CORE");
        assert_eq!(diagnostic["partial"], true);
        assert_eq!(
            diagnostic["shapes"]["attributeCoverage"]["jsonType"],
            "STRING"
        );
        assert_eq!(diagnostic["shapes"]["sourceRowHash"]["jsonType"], "STRING");
    }

    #[test]
    fn relation_schema_hash_and_source_binding_are_tamper_evident() {
        let mut malformed = verified_facts();
        malformed["declarationRelations"]["relations"][0]["provider"] = json!("CALLER");
        refresh(
            &mut malformed,
            "declarationRelations",
            "declarationRelationHash",
        );
        assert_eq!(
            validate_declaration_relation_snapshot(&malformed)
                .unwrap_err()
                .code,
            ErrorCode::InvalidInput
        );

        let mut forged_hash = verified_facts();
        forged_hash["declarationRelationHash"] = json!(format!("sha256:{}", "0".repeat(64)));
        assert_eq!(
            validate_declaration_relation_snapshot(&forged_hash)
                .unwrap_err()
                .code,
            ErrorCode::ProjectModelChanged
        );

        let mut escaped = verified_facts();
        escaped["files"][0]["path"] = json!("../A.kt");
        assert_eq!(
            validate_declaration_descriptor_snapshot(&escaped)
                .unwrap_err()
                .code,
            ErrorCode::ProjectModelChanged
        );
    }
}
