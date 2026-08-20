use crate::canonical;
use crate::error::{ClewError, ErrorCode};
use crate::graph;
use crate::index::{REPOSITORY_INDEX_FACT, RepositoryIndex, StagedIndex};
use crate::model::*;
use crate::proto::RequestKind;
use crate::worker::{WorkerClient, stable_project_model_identity, workspace_root};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use similar::TextDiff;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn preview(
    repo: &Path,
    thread: &ThreadIr,
    edit: &EditIr,
    worker: &mut WorkerClient,
) -> Result<PreviewReport, ClewError> {
    preview_with_authorization(repo, thread, edit, worker, false)
}

pub(crate) fn preview_authorized_semantic_overlay(
    repo: &Path,
    thread: &ThreadIr,
    edit: &EditIr,
    worker: &mut WorkerClient,
) -> Result<PreviewReport, ClewError> {
    preview_with_authorization(repo, thread, edit, worker, true)
}

fn preview_with_authorization(
    repo: &Path,
    thread: &ThreadIr,
    edit: &EditIr,
    worker: &mut WorkerClient,
    allow_authority_semantic_operation: bool,
) -> Result<PreviewReport, ClewError> {
    let head = git_output(repo, &["rev-parse", "HEAD"])?;
    if head != edit.base_revision || head != thread.snapshot.base_revision {
        return Err(ClewError::new(
            ErrorCode::ProjectModelChanged,
            format!(
                "snapshot base {} does not match HEAD {head}",
                edit.base_revision
            ),
        ));
    }
    if edit.thread_id != thread.thread_id {
        return Err(ClewError::new(
            ErrorCode::PreconditionFailed,
            "Edit IR threadId does not match Thread IR",
        ));
    }
    if edit.operations.is_empty() {
        return Err(ClewError::new(
            ErrorCode::InvalidInput,
            "Edit IR has no operations",
        ));
    }
    let mut candidates = BTreeMap::new();
    let mut writes = Vec::new();
    let mut expected_writes = Vec::new();
    let mut windows = Vec::new();
    let mut diagnostics = Vec::new();
    let mut model_input_project = None::<Value>;
    let defer_semantic_validation = edit.operations.iter().any(|operation| {
        matches!(
            operation.kind.as_str(),
            "REPLACE_DECLARATION" | "REWRITE_DECLARATION" | "CREATE_FILE"
        )
    });
    for operation in &edit.operations {
        if operation.kind != "REPLACE_EXPRESSION"
            && operation.kind != "REPLACE_FUNCTION_BODY"
            && operation.kind != "ADD_IMPORT"
            && operation.kind != "REMOVE_IMPORT"
            && operation.kind != "REPLACE_DECLARATION"
            && operation.kind != "REWRITE_DECLARATION"
            && operation.kind != "CREATE_FILE"
            && operation.kind != "REPLACE_MODEL_INPUT"
            && operation.kind != "MAP_EDGE_WITH_CONTEXT"
        {
            return Err(ClewError::new(
                ErrorCode::InvalidInput,
                format!("unsupported edit operation {}", operation.kind),
            ));
        }
        let is_authority_semantic = operation.kind == "MAP_EDGE_WITH_CONTEXT";
        if is_authority_semantic && !allow_authority_semantic_operation {
            return Err(ClewError::new(
                ErrorCode::PreconditionFailed,
                "MAP_EDGE_WITH_CONTEXT can only be applied from a live authority proof receipt",
            ));
        }
        if is_authority_semantic {
            if !operation.replacement.kotlin.is_empty()
                || !matches!(
                    operation.semantic_operation.as_ref(),
                    Some(SemanticOperation::MapEdgeWithContext { .. })
                )
            {
                return Err(ClewError::new(
                    ErrorCode::InvalidInput,
                    "MAP_EDGE_WITH_CONTEXT requires typed arguments and forbids replacement text",
                ));
            }
        } else if operation.semantic_operation.is_some() {
            return Err(ClewError::new(
                ErrorCode::InvalidInput,
                "textual edit kinds cannot carry a semantic operation payload",
            ));
        }
        let target = operation.target.as_object().ok_or_else(|| {
            ClewError::new(
                ErrorCode::InvalidInput,
                "operation target must be an anchor object",
            )
        })?;
        let file = target
            .get("fileId")
            .and_then(Value::as_str)
            .ok_or_else(|| ClewError::new(ErrorCode::InvalidInput, "target has no fileId"))?;
        if operation.kind == "REPLACE_MODEL_INPUT" {
            if model_input_project.is_none() {
                model_input_project = Some(worker.request(
                    RequestKind::OpenProject,
                    &json!({
                        "repo":repo,
                        "compilation":thread.snapshot.compilation,
                    }),
                )?);
            }
            preview_replace_model_input(
                repo,
                thread,
                operation,
                target,
                file,
                model_input_project.as_ref().expect("model input project"),
                &mut candidates,
                &mut writes,
                &mut expected_writes,
                &mut windows,
            )?;
            continue;
        }
        if writes
            .iter()
            .any(|write| write.kind == "MODEL_INPUT" && write.key == file)
        {
            return Err(ClewError::new(
                ErrorCode::InvalidInput,
                format!("model input cannot be combined with another edit kind: {file}"),
            ));
        }
        if operation.kind == "CREATE_FILE" {
            preview_create_file(
                repo,
                operation,
                file,
                &mut candidates,
                &mut writes,
                &mut expected_writes,
                worker,
            )?;
            continue;
        }
        if let Some(expected) = operation
            .preconditions
            .get("nodeTextHash")
            .and_then(Value::as_str)
            && target.get("exactTextHash").and_then(Value::as_str) != Some(expected)
        {
            return Err(ClewError::new(
                ErrorCode::PreconditionFailed,
                "nodeTextHash precondition does not match the target anchor",
            ));
        }
        if let Some(expected) = operation
            .preconditions
            .get("ownerSignatureHash")
            .and_then(Value::as_str)
        {
            let owner = target
                .get("ownerSymbolId")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let resolved = worker.request(
                RequestKind::ResolveSymbol,
                &json!({"repo":repo,"symbol":owner,"compilation":thread.snapshot.compilation}),
            )?;
            if resolved
                .pointer("/declaration/signatureHash")
                .and_then(Value::as_str)
                != Some(expected)
            {
                return Err(ClewError::new(
                    ErrorCode::PreconditionFailed,
                    "ownerSignatureHash precondition failed",
                ));
            }
        }
        let path = safe_join(repo, file)?;
        let original = std::fs::read_to_string(&path).map_err(io_error)?;
        let current = candidates
            .get(file)
            .cloned()
            .unwrap_or_else(|| original.clone());
        let transport_target = apply_edit_target_transport(target);
        let request = json!({
            "repo": repo, "file": file, "source": current,
            "ownerSymbolId": transport_target["ownerSymbolId"],
            "exactTextHash": transport_target["exactTextHash"],
            "syntaxKind": transport_target["syntaxKind"],
            "normalizedTokenHash": transport_target["normalizedTokenHash"],
            "ancestorPathHash": transport_target["ancestorPathHash"],
            "localOrdinal": transport_target["localOrdinal"],
            "leftContextHash": transport_target["leftContextHash"],
            "rightContextHash": transport_target["rightContextHash"],
            "kind": operation.kind, "replacement": operation.replacement.kotlin,
            "semanticOperation": operation.semantic_operation,
            "compilation": thread.snapshot.compilation,
            "deferSemanticValidation":defer_semantic_validation,
            "preconditions": operation.preconditions, "postconditions": operation.postconditions
        });
        let response = worker.request(RequestKind::ApplyEdit, &request)?;
        if is_authority_semantic {
            validate_map_edge_operation_response(operation, &response)?;
        }
        diagnostics.extend(
            response
                .get("diagnostics")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
        );
        let candidate = response
            .get("source")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ClewError::new(
                    ErrorCode::WorkerProtocolMismatch,
                    "candidate response has no source",
                )
            })?
            .to_owned();
        let forbidden = operation
            .postconditions
            .get("mustNotIntroduceEffects")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let introduced = response
            .get("introducedEffects")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if let Some(effect) = introduced.iter().filter_map(Value::as_str).find(|effect| {
            forbidden
                .iter()
                .filter_map(Value::as_str)
                .any(|blocked| blocked == *effect)
        }) {
            return Err(ClewError::new(
                ErrorCode::EffectChanged,
                format!("replacement introduces forbidden effect {effect}"),
            ));
        }
        let range = target.get("rangeHint").cloned().unwrap_or(json!([]));
        windows.push(json!({"file":file,"range":range}));
        candidates.insert(file.to_owned(), candidate.clone());
        let owner = target
            .get("ownerSymbolId")
            .and_then(Value::as_str)
            .unwrap_or(file);
        if operation.kind == "ADD_IMPORT" || operation.kind == "REMOVE_IMPORT" {
            let key = format!(
                "{file}:import:{}",
                operation
                    .replacement
                    .kotlin
                    .trim()
                    .trim_start_matches("import ")
            );
            expected_writes.push(ExpectedWriteFact {
                kind: "IMPORT".into(),
                key: key.clone(),
            });
            writes.push(WriteFact {
                kind: "IMPORT".into(),
                key,
                before_hash: canonical::hash_bytes(current.as_bytes()),
                after_hash: canonical::hash_bytes(candidate.as_bytes()),
            });
        } else if operation.kind == "REPLACE_DECLARATION" || operation.kind == "REWRITE_DECLARATION"
        {
            let owner = target
                .get("ownerSymbolId")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    ClewError::new(
                        ErrorCode::InvalidInput,
                        "declaration edit target has no ownerSymbolId",
                    )
                })?;
            let key = format!("{file}:{owner}");
            expected_writes.extend([
                ExpectedWriteFact {
                    kind: "DECLARATION".into(),
                    key: key.clone(),
                },
                ExpectedWriteFact {
                    kind: "SUMMARY".into(),
                    key: owner.into(),
                },
            ]);
            writes.push(WriteFact {
                kind: "DECLARATION".into(),
                key,
                before_hash: target
                    .get("exactTextHash")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .into(),
                after_hash: canonical::hash_bytes(operation.replacement.kotlin.as_bytes()),
            });
        } else {
            if is_authority_semantic {
                expected_writes.extend([
                    ExpectedWriteFact {
                        kind: "BODY".into(),
                        key: owner.into(),
                    },
                    ExpectedWriteFact {
                        kind: "SUMMARY".into(),
                        key: owner.into(),
                    },
                ]);
            } else {
                let anchor = target
                    .get("anchorId")
                    .and_then(Value::as_str)
                    .unwrap_or(owner)
                    .to_owned();
                expected_writes.extend([
                    ExpectedWriteFact {
                        kind: "TARGET_ANCHOR".into(),
                        key: anchor.clone(),
                    },
                    ExpectedWriteFact {
                        kind: "BODY".into(),
                        key: owner.into(),
                    },
                    ExpectedWriteFact {
                        kind: "SUMMARY".into(),
                        key: owner.into(),
                    },
                ]);
                if operation
                    .postconditions
                    .contains_key("allowedEffectChanges")
                {
                    expected_writes.push(ExpectedWriteFact {
                        kind: "EFFECTS".into(),
                        key: owner.into(),
                    });
                }
                writes.push(WriteFact {
                    kind: "TARGET_ANCHOR".into(),
                    key: anchor,
                    before_hash: target
                        .get("exactTextHash")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .into(),
                    after_hash: canonical::hash_bytes(operation.replacement.kotlin.as_bytes()),
                });
            }
            if let Some(delta) = response.get("semanticDelta").and_then(Value::as_object) {
                for (field, kind) in [
                    ("body", "BODY"),
                    ("signature", "SIGNATURE"),
                    ("abi", "ABI"),
                    ("summary", "SUMMARY"),
                    ("effects", "EFFECTS"),
                ] {
                    let Some(change) = delta.get(field) else {
                        continue;
                    };
                    let before = change
                        .get("beforeHash")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let after = change
                        .get("afterHash")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    if before == after {
                        continue;
                    }
                    if kind == "ABI" || kind == "SIGNATURE" {
                        return Err(ClewError::new(
                            ErrorCode::AbiChanged,
                            format!("edit changes protected {kind} for {owner}"),
                        ));
                    }
                    writes.push(WriteFact {
                        kind: kind.into(),
                        key: change
                            .get("key")
                            .and_then(Value::as_str)
                            .unwrap_or(owner)
                            .into(),
                        before_hash: before.into(),
                        after_hash: after.into(),
                    });
                }
            }
        }
    }
    candidates.retain(|file, candidate| {
        std::fs::read_to_string(safe_join(repo, file).unwrap())
            .map(|original| original != *candidate)
            .unwrap_or(true)
    });
    writes.retain(|write| write.before_hash != write.after_hash);
    if !edit.expected_write_set.is_empty() {
        expected_writes = edit.expected_write_set.clone();
    }
    expected_writes.sort();
    expected_writes.dedup();
    let expected_scope: BTreeSet<_> = expected_writes
        .iter()
        .map(|fact| (&fact.kind, &fact.key))
        .collect();
    if let Some(exceeded) = writes
        .iter()
        .find(|fact| !expected_scope.contains(&(&fact.kind, &fact.key)))
    {
        return Err(ClewError::new(
            ErrorCode::WritesetExceeded,
            format!(
                "actual write {}:{} is outside ExpectedWriteSet",
                exceeded.kind, exceeded.key
            ),
        ));
    }
    let mut diff = String::new();
    for (file, candidate) in &candidates {
        let original = std::fs::read_to_string(safe_join(repo, file)?).unwrap_or_default();
        diff.push_str(&unified_diff(file, &original, candidate));
    }
    writes.sort();
    windows.sort_by_key(|v| v.to_string());
    Ok(PreviewReport {
        schema: "semantic-preview/0.1".into(),
        transaction_id: format!("tx:{}", uuid::Uuid::new_v4()),
        base_revision: head,
        valid: true,
        changed_files: candidates.keys().cloned().collect(),
        diff,
        candidates,
        actual_write_set: writes,
        expected_write_set: expected_writes,
        diagnostics,
        formatting_windows: windows,
    })
}

fn apply_edit_target_transport(target: &serde_json::Map<String, Value>) -> Value {
    let original_owner = target
        .get("ownerSymbolId")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let owner_symbol_id = canonicalize_owner_symbol_query(original_owner)
        .unwrap_or_else(|| original_owner.to_owned());
    json!({
        "ownerSymbolId": owner_symbol_id,
        "exactTextHash": target.get("exactTextHash").and_then(Value::as_str).unwrap_or_default(),
        "syntaxKind": target.get("syntaxKind").and_then(Value::as_str).unwrap_or_default(),
        "normalizedTokenHash": target.get("normalizedTokenHash").and_then(Value::as_str).unwrap_or_default(),
        "ancestorPathHash": target.get("ancestorPathHash").cloned().unwrap_or(Value::Null),
        "localOrdinal": target.get("localOrdinal").cloned().unwrap_or(Value::Null),
        "leftContextHash": target.get("leftContextHash").cloned().unwrap_or(Value::Null),
        "rightContextHash": target.get("rightContextHash").cloned().unwrap_or(Value::Null),
    })
}

fn canonicalize_owner_symbol_query(owner: &str) -> Option<String> {
    let mut identity: Value = serde_json::from_str(owner).ok()?;
    let object = identity.as_object_mut()?;
    for field in [
        "module",
        "sourceSet",
        "package",
        "declarationName",
        "declarationKind",
        "returnType",
        "jvmDescriptor",
    ] {
        if !object
            .get(field)
            .and_then(Value::as_str)
            .is_some_and(|value| !value.is_empty())
        {
            return None;
        }
    }
    if object
        .get("typeParameterArity")
        .and_then(Value::as_u64)
        .is_none()
        || object.get("suspendFlag").and_then(Value::as_bool).is_none()
        || !json_string_array(object.get("containingDeclarations"))
        || !json_string_array(object.get("contextReceiverTypes"))
    {
        return None;
    }

    let mut canonical_types = Vec::new();
    for field in ["parameterTypes", "receiverTypes"] {
        let values = object.get(field)?.as_array()?;
        let values = values
            .iter()
            .map(Value::as_str)
            .map(|value| value.and_then(canonicalize_identity_type_tokens))
            .collect::<Option<Vec<_>>>()?;
        canonical_types.push((field, values));
    }
    for (field, values) in canonical_types {
        object.insert(field.to_owned(), json!(values));
    }
    serde_json::to_string(&identity).ok()
}

fn json_string_array(value: Option<&Value>) -> bool {
    value
        .and_then(Value::as_array)
        .is_some_and(|values| values.iter().all(|value| value.as_str().is_some()))
}

fn canonicalize_identity_type_tokens(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut output = String::with_capacity(value.len());
    let mut cursor = 0usize;
    let mut generic_depth = 0usize;
    let mut expect_type = true;
    let mut saw_type = false;

    while cursor < bytes.len() {
        let byte = bytes[cursor];
        if byte.is_ascii_whitespace() {
            output.push(byte as char);
            cursor += 1;
            continue;
        }
        if is_identity_start(byte) {
            let start = cursor;
            cursor += 1;
            while cursor < bytes.len() && is_identity_continue(bytes[cursor]) {
                cursor += 1;
            }
            let token = &value[start..cursor];
            if !expect_type || !valid_qualified_identity_atom(token) {
                return None;
            }
            if matches!(token, "in" | "out") {
                if cursor == bytes.len() || !bytes[cursor].is_ascii_whitespace() {
                    return None;
                }
                output.push_str(token);
                continue;
            }
            output.push_str(token.rsplit(['/', '.']).next()?);
            expect_type = false;
            saw_type = true;
            continue;
        }
        match byte {
            b'<' if !expect_type => {
                generic_depth += 1;
                expect_type = true;
            }
            b',' if generic_depth > 0 && !expect_type => expect_type = true,
            b'>' if generic_depth > 0 && !expect_type => {
                generic_depth -= 1;
                expect_type = false;
            }
            b'*' if expect_type => {
                expect_type = false;
                saw_type = true;
            }
            b'?' | b'!' if !expect_type => {}
            b'&' if !expect_type => expect_type = true,
            _ => return None,
        }
        output.push(byte as char);
        cursor += 1;
    }
    (saw_type && !expect_type && generic_depth == 0).then_some(output)
}

fn is_identity_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || matches!(byte, b'_' | b'$')
}

fn is_identity_continue(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'$' | b'/' | b'.')
}

fn valid_qualified_identity_atom(value: &str) -> bool {
    value.split('/').all(|segment| {
        !segment.is_empty()
            && segment.split('.').all(|part| {
                part.as_bytes()
                    .first()
                    .is_some_and(|byte| is_identity_start(*byte))
                    && part.as_bytes()[1..]
                        .iter()
                        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'$'))
            })
    })
}

#[allow(clippy::too_many_arguments)]
fn preview_replace_model_input(
    repo: &Path,
    thread: &ThreadIr,
    operation: &EditOperation,
    target: &serde_json::Map<String, Value>,
    file: &str,
    project: &Value,
    candidates: &mut BTreeMap<String, String>,
    writes: &mut Vec<WriteFact>,
    expected_writes: &mut Vec<ExpectedWriteFact>,
    windows: &mut Vec<Value>,
) -> Result<(), ClewError> {
    if target.get("syntaxKind").and_then(Value::as_str) != Some("MODEL_INPUT_FILE") {
        return Err(ClewError::new(
            ErrorCode::InvalidInput,
            "REPLACE_MODEL_INPUT requires an emitted model-input target",
        ));
    }
    if candidates.contains_key(file) {
        return Err(ClewError::new(
            ErrorCode::InvalidInput,
            format!("model input may be replaced only once: {file}"),
        ));
    }
    require_canonical_tracked_model_input(repo, file)?;
    let expected_hash = target
        .get("exactTextHash")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            ClewError::new(
                ErrorCode::InvalidInput,
                "model-input target has no exactTextHash",
            )
        })?;
    validate_model_input_project_authority(
        project,
        &thread.snapshot.project_model_hash,
        target
            .get("semanticInputManifestHash")
            .and_then(Value::as_str),
        file,
        expected_hash,
    )?;
    let bytes = std::fs::read(safe_join(repo, file)?).map_err(io_error)?;
    let current = std::str::from_utf8(&bytes).map_err(|_| {
        ClewError::new(
            ErrorCode::InvalidInput,
            format!("model input is not UTF-8: {file}"),
        )
    })?;
    let current_hash = canonical::hash_bytes(&bytes);
    if current_hash != expected_hash {
        return Err(ClewError::new(
            ErrorCode::WwConflict,
            format!("model input changed after context capture: {file}"),
        ));
    }
    let candidate = operation.replacement.kotlin.clone();
    let after_hash = canonical::hash_bytes(candidate.as_bytes());
    candidates.insert(file.to_owned(), candidate);
    expected_writes.push(ExpectedWriteFact {
        kind: "MODEL_INPUT".into(),
        key: file.into(),
    });
    writes.push(WriteFact {
        kind: "MODEL_INPUT".into(),
        key: file.into(),
        before_hash: current_hash,
        after_hash,
    });
    windows.push(json!({
        "file":file,
        "range":"WHOLE_FILE",
        "beforeBytes":current.len(),
        "afterBytes":operation.replacement.kotlin.len(),
    }));
    Ok(())
}

fn validate_model_input_project_authority(
    project: &Value,
    expected_project_model_hash: &str,
    expected_manifest_hash: Option<&str>,
    file: &str,
    expected_file_hash: &str,
) -> Result<(), ClewError> {
    if project.get("projectModelHash").and_then(Value::as_str) != Some(expected_project_model_hash)
    {
        return Err(ClewError::new(
            ErrorCode::ProjectModelChanged,
            "live project model differs from model-input task authority",
        ));
    }
    let (manifest, manifest_hash) = verified_manifest(project)?;
    if expected_manifest_hash != Some(manifest_hash) {
        return Err(ClewError::new(
            ErrorCode::ProjectModelChanged,
            "model-input target manifest differs from live OpenProject",
        ));
    }
    require_model_input_digest(manifest, file, expected_file_hash)
}

fn verified_manifest(project: &Value) -> Result<(&Value, &str), ClewError> {
    let manifest = project.get("semanticInputManifest").ok_or_else(|| {
        ClewError::new(
            ErrorCode::WorkerProtocolMismatch,
            "OpenProject has no semantic input manifest",
        )
    })?;
    let manifest_hash = project
        .get("semanticInputManifestHash")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            ClewError::new(
                ErrorCode::WorkerProtocolMismatch,
                "OpenProject has no semantic input manifest hash",
            )
        })?;
    if canonical::hash(manifest).map_err(internal)? != manifest_hash {
        return Err(ClewError::new(
            ErrorCode::ProjectModelChanged,
            "OpenProject semantic input manifest hash is invalid",
        ));
    }
    Ok((manifest, manifest_hash))
}

fn require_model_input_digest(
    manifest: &Value,
    file: &str,
    expected_hash: &str,
) -> Result<(), ClewError> {
    let entries = manifest
        .get("modelInputs")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            ClewError::new(
                ErrorCode::WorkerProtocolMismatch,
                "semantic input manifest has no modelInputs array",
            )
        })?;
    let matches = entries
        .iter()
        .filter(|entry| entry.get("path").and_then(Value::as_str) == Some(file))
        .collect::<Vec<_>>();
    if matches.len() != 1 || matches[0].get("hash").and_then(Value::as_str) != Some(expected_hash) {
        return Err(ClewError::new(
            ErrorCode::ProjectModelChanged,
            format!("model input {file} is absent or has a different OpenProject digest"),
        ));
    }
    Ok(())
}

fn require_canonical_tracked_model_input(repo: &Path, relative: &str) -> Result<(), ClewError> {
    let path = Path::new(relative);
    let canonical = !relative.is_empty()
        && !relative.contains('\\')
        && !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
        && path
            .components()
            .map(|component| component.as_os_str())
            .collect::<PathBuf>()
            .as_os_str()
            == std::ffi::OsStr::new(relative);
    if !canonical {
        return Err(ClewError::new(
            ErrorCode::InvalidInput,
            format!("model input path is not canonical: {relative}"),
        ));
    }
    let output = Command::new("git")
        .args(["ls-files", "--stage", "-z", "--", relative])
        .current_dir(repo)
        .output()
        .map_err(io_error)?;
    let records = output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
        .collect::<Vec<_>>();
    let tracked_regular = output.status.success()
        && records.len() == 1
        && records[0]
            .iter()
            .position(|byte| *byte == b'\t')
            .is_some_and(|separator| {
                let (stage, path_with_tab) = records[0].split_at(separator);
                let mut fields = stage.split(|byte| *byte == b' ');
                matches!(fields.next(), Some(b"100644" | b"100755"))
                    && fields.next().is_some()
                    && fields.next() == Some(b"0")
                    && fields.next().is_none()
                    && &path_with_tab[1..] == relative.as_bytes()
            });
    if !tracked_regular {
        return Err(ClewError::new(
            ErrorCode::InvalidInput,
            format!("model input is not one exact tracked regular file: {relative}"),
        ));
    }
    let mut current = repo.to_path_buf();
    let components = path.components().collect::<Vec<_>>();
    for (index, component) in components.iter().enumerate() {
        current.push(component.as_os_str());
        let metadata = std::fs::symlink_metadata(&current).map_err(io_error)?;
        if metadata.file_type().is_symlink()
            || (index + 1 == components.len() && !metadata.is_file())
            || (index + 1 < components.len() && !metadata.is_dir())
        {
            return Err(ClewError::new(
                ErrorCode::InvalidInput,
                format!("model input is not a nonsymlink regular file: {relative}"),
            ));
        }
    }
    Ok(())
}

fn validate_map_edge_operation_response(
    operation: &EditOperation,
    response: &Value,
) -> Result<(), ClewError> {
    let Some(SemanticOperation::MapEdgeWithContext {
        workflow_symbol,
        context_producer_symbol,
        transformer_symbol,
        ..
    }) = operation.semantic_operation.as_ref()
    else {
        return Err(ClewError::new(
            ErrorCode::InvalidInput,
            "map-edge edit has no typed operation payload",
        ));
    };
    if response.get("k2Validated").and_then(Value::as_bool) != Some(true)
        || response
            .get("introducedEffects")
            .and_then(Value::as_array)
            .is_none_or(|effects| !effects.is_empty())
    {
        return Err(ClewError::new(
            ErrorCode::IncompleteSemanticAnalysis,
            "semantic map-edge candidate lacks clean K2/effect evidence",
        ));
    }
    let proof = response
        .get("semanticOperationProof")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            ClewError::new(
                ErrorCode::IncompleteSemanticAnalysis,
                "worker returned no semantic operation proof",
            )
        })?;
    for (field, expected) in [
        ("kind", "MAP_EDGE_WITH_CONTEXT"),
        ("workflowSymbol", workflow_symbol.as_str()),
        ("contextProducerSymbol", context_producer_symbol.as_str()),
        ("transformerSymbol", transformer_symbol.as_str()),
    ] {
        if proof.get(field).and_then(Value::as_str) != Some(expected) {
            return Err(ClewError::new(
                ErrorCode::PreconditionFailed,
                format!("semantic operation proof has wrong {field}"),
            ));
        }
    }
    for invariant in [
        "typeAssignable",
        "contextEvaluatedOnce",
        "placementDominatesUses",
        "orderPreserved",
        "cardinalityPreserved",
        "lazinessPreserved",
        "effectsPreserved",
        "nullabilityPreserved",
        "consumerContractPreserved",
        "abiPreserved",
        "behavioralOracleRequired",
        "noUnsupportedBoundary",
    ] {
        if proof.get(invariant).and_then(Value::as_bool) != Some(true) {
            return Err(ClewError::new(
                ErrorCode::IncompleteSemanticAnalysis,
                format!("semantic operation did not prove {invariant}"),
            ));
        }
    }
    Ok(())
}

fn preview_create_file(
    repo: &Path,
    operation: &EditOperation,
    file: &str,
    candidates: &mut BTreeMap<String, String>,
    writes: &mut Vec<WriteFact>,
    expected_writes: &mut Vec<ExpectedWriteFact>,
    worker: &mut WorkerClient,
) -> Result<(), ClewError> {
    if !file.ends_with(".kt") {
        return Err(ClewError::new(
            ErrorCode::InvalidInput,
            "CREATE_FILE only supports Kotlin .kt files",
        ));
    }
    let path = safe_join(repo, file)?;
    if path.exists() || candidates.contains_key(file) {
        return Err(ClewError::new(
            ErrorCode::PreconditionFailed,
            format!("CREATE_FILE target already exists: {file}"),
        ));
    }
    if operation.replacement.kotlin.trim().is_empty() {
        return Err(ClewError::new(
            ErrorCode::InvalidInput,
            "CREATE_FILE replacement must not be empty",
        ));
    }
    let validation = worker.request(
        RequestKind::ValidateCandidate,
        &json!({"repo":repo,"file":file,"source":operation.replacement.kotlin}),
    )?;
    if validation.get("valid").and_then(Value::as_bool) != Some(true) {
        return Err(ClewError::new(
            ErrorCode::ReplacementParseError,
            format!(
                "CREATE_FILE Kotlin syntax is invalid: {}",
                validation["diagnostics"]
            ),
        ));
    }
    candidates.insert(file.into(), operation.replacement.kotlin.clone());
    expected_writes.push(ExpectedWriteFact {
        kind: "FILE".into(),
        key: file.into(),
    });
    writes.push(WriteFact {
        kind: "FILE".into(),
        key: file.into(),
        before_hash: canonical::hash_bytes(&[]),
        after_hash: canonical::hash_bytes(operation.replacement.kotlin.as_bytes()),
    });
    Ok(())
}

pub fn commit(
    repo: &Path,
    transaction: &mut Transaction,
    target_ref: &str,
    worker: &mut WorkerClient,
) -> Result<Value, ClewError> {
    commit_with_authorization(repo, transaction, target_ref, worker, false)
}

pub(crate) fn commit_authorized_semantic(
    repo: &Path,
    transaction: &mut Transaction,
    target_ref: &str,
    worker: &mut WorkerClient,
) -> Result<Value, ClewError> {
    commit_with_authorization(repo, transaction, target_ref, worker, true)
}

fn commit_with_authorization(
    repo: &Path,
    transaction: &mut Transaction,
    target_ref: &str,
    worker: &mut WorkerClient,
    allow_authority_semantic_operation: bool,
) -> Result<Value, ClewError> {
    validate_required_threads(transaction)?;
    let qualified_target_ref;
    let target_ref = if target_ref.starts_with("refs/") {
        target_ref
    } else {
        qualified_target_ref = format!("refs/heads/{target_ref}");
        &qualified_target_ref
    };
    let current = git_output(repo, &["rev-parse", target_ref])?;
    if allow_authority_semantic_operation && current != transaction.base_revision {
        return Err(ClewError::new(
            ErrorCode::StaleRequiresReslice,
            "authority semantic proof is bound to its exact target revision; re-prove on the current target",
        ));
    }
    let checked_out_target_is_clean = checked_out_target_is_clean(repo, target_ref, &current);
    transaction.target_ref = Some(target_ref.into());
    let base_index_snapshot = transaction
        .base_index_snapshot
        .clone()
        .filter(|snapshot| !snapshot.is_empty())
        .or_else(|| {
            (!transaction.thread.snapshot.index_snapshot.is_empty())
                .then(|| transaction.thread.snapshot.index_snapshot.clone())
        })
        .ok_or_else(|| {
            ClewError::new(
                ErrorCode::InvalidInput,
                "transaction must start with an immutable repository index snapshot",
            )
        })?;
    transaction.base_index_snapshot = Some(base_index_snapshot.clone());
    let current_repository_index =
        RepositoryIndex::open_compilation(repo, Some(&transaction.thread.snapshot.compilation))?;
    current_repository_index.require_fresh(REPOSITORY_INDEX_FACT)?;
    let current_index_snapshot = current_repository_index.hash()?;
    if current == transaction.base_revision
        && current_index_snapshot.as_deref() != Some(base_index_snapshot.as_str())
    {
        return Err(ClewError::new(
            ErrorCode::StaleRequiresReslice,
            format!(
                "repository index snapshot changed since transaction start: expected {base_index_snapshot}, current {}",
                current_index_snapshot.as_deref().unwrap_or("<missing>")
            ),
        ));
    }
    let edit_hash = canonical::hash(&transaction.edit).map_err(internal)?;
    if let Some(existing) =
        find_matching_transaction_commit(repo, target_ref, &transaction.tx_id, &edit_hash)?
    {
        let compilation = &transaction.thread.snapshot.compilation;
        let repository_index = RepositoryIndex::open_compilation(repo, Some(compilation))?;
        let (final_index_snapshot, invalidations) =
            if repository_index.published_revision()?.as_deref() == Some(current.as_str()) {
                (
                    repository_index.hash()?.ok_or_else(|| {
                        ClewError::new(
                            ErrorCode::TransactionRecoveryRequired,
                            "published revision has no repository index hash",
                        )
                    })?,
                    repository_index.invalidations()?,
                )
            } else {
                // The transaction commit may now be an ancestor of the target
                // ref. The repository index always follows the current target,
                // never the older idempotent transaction commit.
                publish_index_for_revision(
                    repo,
                    &current,
                    compilation,
                    transaction.thread.snapshot.build_system,
                    worker,
                )?
            };
        transaction.final_commit = Some(existing.clone());
        transaction.status = "COMMITTED".into();
        let ledger_recorded = ledger(repo)
            .and_then(|ledger| {
                ledger.append(
                    transaction,
                    "idempotent retry matched reachable Git trailers",
                )
            })
            .is_ok();
        return Ok(
            json!({"schema":"semantic-commit/0.1","transactionId":transaction.tx_id,"baseRevision":transaction.base_revision,"finalCommit":existing,"currentRevision":current,"finalIndexSnapshot":final_index_snapshot,"appliedInvalidations":invalidations,"targetRef":target_ref,"status":"COMMITTED","idempotent":true,"ledgerRecorded":ledger_recorded}),
        );
    }
    let report = preview_for_commit(
        repo,
        transaction,
        &current,
        worker,
        allow_authority_semantic_operation,
    )
    .map_err(|mut e| {
        if current != transaction.base_revision
            && matches!(
                e.code,
                ErrorCode::StaleTarget
                    | ErrorCode::AmbiguousTarget
                    | ErrorCode::ProjectModelChanged
            )
        {
            e.code = ErrorCode::WwConflict;
        }
        e
    })?;
    transaction.preview = Some(report.clone());
    transaction.expected_write_set_hash =
        Some(canonical::hash(&report.expected_write_set).map_err(internal)?);
    transaction.actual_write_set_hash =
        Some(canonical::hash(&report.actual_write_set).map_err(internal)?);
    transaction.validation_evidence.push(json!({
        "kind":"SEMANTIC_PREVIEW",
        "valid":report.valid,
        "diagnosticsHash":canonical::hash(&report.diagnostics).map_err(internal)?,
        "expectedWriteSetHash":transaction.expected_write_set_hash,
        "actualWriteSetHash":transaction.actual_write_set_hash
    }));
    transaction.status = "VALIDATED".into();
    ledger(repo)?.append(transaction, "preview and Gradle validation passed")?;
    if report.candidates.is_empty() {
        transaction.final_commit = Some(current.clone());
        transaction.status = "COMMITTED".into();
        ledger(repo)?.append(transaction, "idempotent no-op merged at current ref")?;
        return Ok(
            json!({"schema":"semantic-commit/0.1","transactionId":transaction.tx_id,"baseRevision":current,"finalCommit":current,"targetRef":target_ref,"status":"COMMITTED","idempotent":true}),
        );
    }
    let worktree = tempfile::tempdir().map_err(io_error)?;
    let worktree_path = worktree.path().join("worktree");
    git(
        repo,
        &[
            "worktree",
            "add",
            "--detach",
            worktree_path.to_str().unwrap(),
            &current,
        ],
    )?;
    let result = (|| {
        prepare_candidate_repository_state(
            repo,
            &worktree_path,
            transaction.thread.snapshot.build_system,
        )?;
        for (file, source) in &report.candidates {
            let path = safe_join(&worktree_path, file)?;
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(io_error)?;
            }
            std::fs::write(path, source.as_bytes()).map_err(io_error)?;
        }
        let configured_test_tasks = if transaction.test_tasks.is_empty() {
            &transaction.thread.snapshot.test_tasks
        } else {
            &transaction.test_tasks
        };
        let (compile_duration_ms, test_duration_ms) = validate_worktree(
            &worktree_path,
            transaction.thread.snapshot.build_system,
            &transaction.thread.snapshot.build_launcher,
            &transaction.thread.snapshot.compile_task,
            configured_test_tasks,
        )?;
        transaction.validation_evidence.push(json!({
            "kind":"BUILD",
            "buildSystem":transaction.thread.snapshot.build_system,
            "buildLauncher":transaction.thread.snapshot.build_launcher,
            "compileTask":transaction.thread.snapshot.compile_task,
            "testTasks":configured_test_tasks,
            "compileDurationMs":compile_duration_ms,
            "testDurationMs":test_duration_ms,
            "compileCoveredByTestLifecycle": transaction.thread.snapshot.build_system == BuildSystem::Maven && !configured_test_tasks.is_empty(),
            "status":"PASSED"
        }));
        let mut add_args = vec!["add", "--"];
        add_args.extend(report.candidates.keys().map(String::as_str));
        git(&worktree_path, &add_args)?;
        let message = format!(
            "semantic transaction {}\n\nSemantic-Transaction-Id: {}\nSemantic-Base-Revision: {}\nSemantic-Edit-Hash: {}",
            transaction.intent, transaction.tx_id, transaction.base_revision, edit_hash
        );
        let output = Command::new("git")
            .args([
                "-c",
                "user.name=Codeclew",
                "-c",
                "user.email=codeclew@localhost",
                "commit",
                "-m",
                &message,
            ])
            .current_dir(&worktree_path)
            .output()
            .map_err(io_error)?;
        if !output.status.success() {
            log_output(&output);
            return Err(ClewError::new(
                ErrorCode::Internal,
                "candidate commit failed",
            ));
        }
        let candidate = git_output(&worktree_path, &["rev-parse", "HEAD"])?;
        transaction.candidate_commit = Some(candidate.clone());
        transaction.status = "COMMITTING".into();
        ledger(repo)?.append(transaction, "candidate commit created")?;
        let authority_model = worker.request(
            RequestKind::OpenProject,
            &json!({
                "repo":repo,
                "compilation":transaction.thread.snapshot.compilation,
            }),
        )?;
        if authority_model
            .get("projectModelHash")
            .and_then(Value::as_str)
            != Some(transaction.project_model_hash.as_str())
        {
            return Err(ClewError::new(
                ErrorCode::ProjectModelChanged,
                "live project model differs from transaction authority",
            ));
        }
        let candidate_model = worker.request(
            RequestKind::OpenProject,
            &json!({
                "repo":worktree_path,
                "compilation":transaction.thread.snapshot.compilation,
            }),
        )?;
        let mut project_model_transition = project_model_transition_evidence(
            &authority_model,
            &candidate_model,
            &report.actual_write_set,
        )?;
        let index_facts = worker.index_files_verified(&json!({
            "repo":worktree_path,
            "compilation":transaction.thread.snapshot.compilation,
            "syntaxOnly":false
        }))?;
        if candidate_model
            .get("projectModelHash")
            .and_then(Value::as_str)
            != Some(index_facts.project_model_hash())
        {
            return Err(ClewError::new(
                ErrorCode::ProjectModelChanged,
                "candidate index project model differs from its live authority",
            ));
        }
        if let Some(transition) = project_model_transition.as_mut() {
            transition["candidateIndexProjectModelHash"] = json!(index_facts.project_model_hash());
            transition["status"] = json!("VERIFIED");
        }
        let staged_index = RepositoryIndex::stage_update(
            repo,
            Some(&transaction.thread.snapshot.compilation),
            &index_facts,
            worker,
            &worktree_path,
            &candidate,
        )?;
        if let Some(transition) = project_model_transition {
            transaction.validation_evidence.push(transition);
        }
        git(repo, &["update-ref", target_ref, &candidate, &current]).map_err(|_| {
            ClewError::new(
                ErrorCode::RefCompareAndSwapFailed,
                "target ref changed during commit CAS",
            )
        })?;
        let (final_index_snapshot, invalidations) = match staged_index.publish() {
            Ok(published) => published,
            Err(publication_error) => {
                if git(repo, &["update-ref", target_ref, &current, &candidate]).is_ok() {
                    return Err(ClewError::new(
                        ErrorCode::Internal,
                        format!(
                            "repository index publication failed; target ref was rolled back: {}",
                            publication_error.message
                        ),
                    ));
                }
                return Err(index_recovery_error(ClewError::new(
                    ErrorCode::TransactionRecoveryRequired,
                    format!(
                        "index publication failed and target ref rollback also failed: {}",
                        publication_error.message
                    ),
                )));
            }
        };
        let worktree_synchronized = if checked_out_target_is_clean {
            // update-ref intentionally does not touch the caller's index or
            // worktree. The pre-publication cleanliness check makes this
            // synchronization safe and prevents a successful transaction from
            // looking like a staged reverse diff to the next agent.
            git(repo, &["reset", "--hard", &candidate]).is_ok()
        } else {
            false
        };
        transaction.validation_evidence.push(json!({
            "kind":"INDEX_PUBLICATION",
            "baseIndexSnapshot":base_index_snapshot,
            "finalIndexSnapshot":final_index_snapshot,
            "appliedInvalidations":invalidations,
            "worktreeSynchronized":worktree_synchronized
        }));
        transaction.final_commit = Some(candidate.clone());
        transaction.status = "COMMITTED".into();
        // Ref + index publication is the commit point. A later ledger write
        // cannot turn that committed outcome into a reported failed
        // transaction; Git trailers let inspection reconstruct the event.
        let ledger_recorded = ledger(repo)
            .and_then(|ledger| {
                ledger.append(transaction, "target ref and index updated atomically")
            })
            .is_ok();
        Ok(
            json!({"schema":"semantic-commit/0.1","transactionId":transaction.tx_id,"baseRevision":current,"baseIndexSnapshot":base_index_snapshot,"finalCommit":candidate,"finalIndexSnapshot":final_index_snapshot,"appliedInvalidations":invalidations,"targetRef":target_ref,"status":"COMMITTED","gradleValidationDurationMs":compile_duration_ms + test_duration_ms,"ledgerRecorded":ledger_recorded}),
        )
    })();
    let _ = git(
        repo,
        &[
            "worktree",
            "remove",
            "--force",
            worktree_path.to_str().unwrap(),
        ],
    );
    result
}

fn project_model_transition_evidence(
    authority: &Value,
    candidate: &Value,
    actual_writes: &[WriteFact],
) -> Result<Option<Value>, ClewError> {
    let before_stable = stable_project_model_identity(authority)?;
    let after_stable = stable_project_model_identity(candidate)?;
    let model_writes = actual_writes
        .iter()
        .filter(|write| write.kind == "MODEL_INPUT")
        .collect::<Vec<_>>();
    if model_writes.is_empty() {
        if before_stable != after_stable {
            return Err(ClewError::new(
                ErrorCode::ProjectModelChanged,
                "candidate semantic project model differs without an authorized model-input write",
            ));
        }
        return Ok(None);
    }

    let (before_manifest, before_manifest_hash) = verified_manifest(authority)?;
    let (after_manifest, after_manifest_hash) = verified_manifest(candidate)?;
    let mut seen = BTreeSet::new();
    let mut transitions = Vec::with_capacity(model_writes.len());
    for write in model_writes {
        if !seen.insert(write.key.as_str()) || write.before_hash == write.after_hash {
            return Err(ClewError::new(
                ErrorCode::ProjectModelChanged,
                "candidate model transition has an invalid MODEL_INPUT write fact",
            ));
        }
        require_model_input_digest(before_manifest, &write.key, &write.before_hash)?;
        require_model_input_digest(after_manifest, &write.key, &write.after_hash)?;
        transitions.push(json!({
            "path":write.key,
            "beforeHash":write.before_hash,
            "afterHash":write.after_hash,
        }));
    }
    let stable_identity_changed = before_stable != after_stable;
    Ok(Some(json!({
        "kind":"PROJECT_MODEL_TRANSITION",
        "authority":"REPLACE_MODEL_INPUT",
        "status":"PENDING_INDEX_VERIFICATION",
        "beforeProjectModelHash":authority.get("projectModelHash"),
        "afterProjectModelHash":candidate.get("projectModelHash"),
        "beforeSemanticInputManifestHash":before_manifest_hash,
        "afterSemanticInputManifestHash":after_manifest_hash,
        "beforeStableProjectModelIdentity":before_stable,
        "afterStableProjectModelIdentity":after_stable,
        "stableIdentityChanged":stable_identity_changed,
        "modelInputs":transitions,
    })))
}

fn publish_index_for_revision(
    repo: &Path,
    revision: &str,
    compilation: &str,
    build_system: BuildSystem,
    worker: &mut WorkerClient,
) -> Result<(String, Vec<String>), ClewError> {
    stage_index_for_revision(repo, revision, compilation, build_system, worker)
        .and_then(StagedIndex::publish)
        .map_err(index_recovery_error)
}

fn stage_index_for_revision(
    repo: &Path,
    revision: &str,
    compilation: &str,
    build_system: BuildSystem,
    worker: &mut WorkerClient,
) -> Result<StagedIndex, ClewError> {
    let temporary = tempfile::tempdir().map_err(io_error)?;
    let path = temporary.path().join("index-recovery");
    git(
        repo,
        &[
            "worktree",
            "add",
            "--detach",
            path.to_str().unwrap(),
            revision,
        ],
    )?;
    let result = (|| {
        prepare_candidate_repository_state(repo, &path, build_system)?;
        let facts = worker.index_files_verified(&json!({"repo":path,"compilation":compilation}))?;
        RepositoryIndex::stage_update(repo, Some(compilation), &facts, worker, &path, revision)
    })()
    .map_err(index_recovery_error);
    let _ = git(
        repo,
        &["worktree", "remove", "--force", path.to_str().unwrap()],
    );
    result
}

fn index_recovery_error(mut error: ClewError) -> ClewError {
    error.code = ErrorCode::TransactionRecoveryRequired;
    error.retryable = true;
    error.message = format!(
        "repository index publication requires recovery: {}",
        error.message
    );
    error
}

fn preview_for_commit(
    repo: &Path,
    transaction: &mut Transaction,
    current: &str,
    worker: &mut WorkerClient,
    allow_authority_semantic_operation: bool,
) -> Result<PreviewReport, ClewError> {
    if current == transaction.base_revision {
        return preview_with_authorization(
            repo,
            &transaction.thread,
            &transaction.edit,
            worker,
            allow_authority_semantic_operation,
        );
    }
    transaction.status = "REBASING".into();
    ledger(repo)?.append(transaction, "target ref moved; semantic replay requested")?;
    let temp = tempfile::tempdir().map_err(io_error)?;
    let replay_path = temp.path().join("replay");
    git(
        repo,
        &[
            "worktree",
            "add",
            "--detach",
            replay_path.to_str().unwrap(),
            current,
        ],
    )?;
    let result = (|| {
        prepare_candidate_repository_state(
            repo,
            &replay_path,
            transaction.thread.snapshot.build_system,
        )?;
        let current_model = worker.request(
            RequestKind::OpenProject,
            &json!({"repo":replay_path,"compilation":transaction.thread.snapshot.compilation}),
        )?;
        if current_model
            .get("projectModelHash")
            .and_then(Value::as_str)
            != Some(transaction.project_model_hash.as_str())
        {
            return Err(ClewError::new(
                ErrorCode::StaleRequiresReslice,
                format!(
                    "project model changed since slice: expected {}, current {}",
                    transaction.project_model_hash,
                    current_model
                        .get("projectModelHash")
                        .and_then(Value::as_str)
                        .unwrap_or("<missing>")
                ),
            ));
        }
        revalidate_semantic_read_set(&replay_path, transaction, &current_model, current, worker)?;
        let mut replay_thread = transaction.thread.clone();
        let mut replay_edit = transaction.edit.clone();
        replay_thread.snapshot.base_revision = current.to_owned();
        replay_edit.base_revision = current.to_owned();
        preview_with_authorization(
            &replay_path,
            &replay_thread,
            &replay_edit,
            worker,
            allow_authority_semantic_operation,
        )
    })();
    let _ = git(
        repo,
        &[
            "worktree",
            "remove",
            "--force",
            replay_path.to_str().unwrap(),
        ],
    );
    if matches!(&result, Err(error) if error.code == ErrorCode::StaleRequiresReslice) {
        transaction.status = "STALE_REQUIRES_RESLICE".into();
        let _ = ledger(repo)?.append(transaction, "project model or read dependency changed");
    }
    result
}

fn revalidate_semantic_read_set(
    repo: &Path,
    transaction: &Transaction,
    project: &Value,
    current: &str,
    worker: &mut WorkerClient,
) -> Result<(), ClewError> {
    let required = transaction_threads(transaction);
    let rebuilt = required
        .iter()
        .map(|thread| rebuild_thread(repo, thread, project, current, worker))
        .collect::<Result<Vec<_>, _>>()?;

    let mut old_union = BTreeSet::new();
    let mut new_union = BTreeSet::new();
    for (old_thread, new_thread) in required.iter().zip(&rebuilt) {
        let old = semantic_read_set(old_thread);
        let new = semantic_read_set(new_thread);
        old_union.extend(old.iter().cloned());
        new_union.extend(new.iter().cloned());
        if old != new {
            return Err(read_set_change_error(
                repo,
                transaction,
                &old,
                &new,
                Some(&old_thread.thread_id),
            ));
        }
    }
    if old_union != new_union {
        return Err(read_set_change_error(
            repo,
            transaction,
            &old_union,
            &new_union,
            None,
        ));
    }
    Ok(())
}

pub(crate) fn rebuild_thread(
    repo: &Path,
    thread: &ThreadIr,
    project: &Value,
    current: &str,
    worker: &mut WorkerClient,
) -> Result<ThreadIr, ClewError> {
    let symbol = thread
        .seed
        .get("symbol")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            ClewError::new(
                ErrorCode::StaleRequiresReslice,
                "thread seed has no owner symbol",
            )
        })?;
    let raw = worker.request(
        RequestKind::BuildLocalGraph,
        &json!({"repo":repo,"symbol":symbol,"compilation":thread.snapshot.compilation}),
    )?;
    let graph =
        graph::enrich(serde_json::from_value::<LocalGraph>(raw).map_err(|error| {
            ClewError::new(ErrorCode::WorkerProtocolMismatch, error.to_string())
        })?);
    let old_seed_id = thread.seed.get("nodeId").and_then(Value::as_str);
    let seed_anchor = thread
        .seed
        .get("anchor")
        .and_then(|anchor| anchor.get("anchorId"))
        .and_then(Value::as_str);
    let seed_id = old_seed_id
        .filter(|id| graph.nodes.iter().any(|node| node.id == *id))
        .map(str::to_owned)
        .or_else(|| {
            graph
                .nodes
                .iter()
                .find(|node| {
                    node.origin
                        .as_ref()
                        .and_then(|origin| origin.get("anchorId"))
                        .and_then(Value::as_str)
                        == seed_anchor
                })
                .map(|node| node.id.clone())
        })
        .ok_or_else(|| {
            ClewError::new(
                ErrorCode::StaleRequiresReslice,
                "slice seed no longer resolves",
            )
        })?;
    let snapshot = Snapshot {
        base_revision: current.into(),
        project_model_hash: project
            .get("projectModelHash")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .into(),
        compiler_version: thread.snapshot.compiler_version.clone(),
        build_system: thread.snapshot.build_system,
        build_launcher: thread.snapshot.build_launcher.clone(),
        index_snapshot: thread.snapshot.index_snapshot.clone(),
        compilation: thread.snapshot.compilation.clone(),
        compile_task: thread.snapshot.compile_task.clone(),
        test_tasks: thread.snapshot.test_tasks.clone(),
    };
    graph::slice(
        &graph,
        &seed_id,
        thread.policy.clone(),
        snapshot,
        thread.seed.clone(),
    )
    .map_err(internal)
}

fn semantic_read_set(thread: &ThreadIr) -> BTreeSet<ReadFact> {
    thread
        .read_set
        .iter()
        .filter(|fact| fact.kind != "PROJECT_MODEL")
        .cloned()
        .collect()
}

fn read_set_change_error(
    repo: &Path,
    transaction: &Transaction,
    old: &BTreeSet<ReadFact>,
    new: &BTreeSet<ReadFact>,
    thread_id: Option<&str>,
) -> ClewError {
    let removed: Vec<_> = old
        .difference(new)
        .take(8)
        .map(|fact| format!("- {} {} {}", fact.kind, fact.key, fact.hash))
        .collect();
    let added: Vec<_> = new
        .difference(old)
        .take(8)
        .map(|fact| format!("+ {} {} {}", fact.kind, fact.key, fact.hash))
        .collect();
    let target_anchors: BTreeSet<_> = transaction
        .edit
        .operations
        .iter()
        .filter_map(|operation| operation.target.get("anchorId").and_then(Value::as_str))
        .collect();
    let target_text_changed = transaction.edit.operations.iter().any(|operation| {
        let Some(file) = operation.target.get("fileId").and_then(Value::as_str) else {
            return true;
        };
        let Some(text) = operation.target.get("sourceText").and_then(Value::as_str) else {
            return true;
        };
        std::fs::read_to_string(repo.join(file)).map_or(true, |source| !source.contains(text))
    });
    let write_conflict = target_text_changed
        || old
            .difference(new)
            .any(|fact| fact.kind == "SOURCE_NODE" && target_anchors.contains(fact.key.as_str()));
    let mut error = ClewError::new(
        if write_conflict {
            ErrorCode::WwConflict
        } else {
            ErrorCode::StaleRequiresReslice
        },
        if write_conflict {
            "concurrent write changed the target anchor"
        } else {
            "semantic ReadSet changed since slice"
        },
    );
    if let Some(thread_id) = thread_id {
        error.evidence.push(format!("requiredThreadId={thread_id}"));
    } else {
        error
            .evidence
            .push("requiredThreadUnionChanged=true".into());
    }
    error.evidence.extend(removed.into_iter().chain(added));
    error
}

fn transaction_threads(transaction: &Transaction) -> Vec<&ThreadIr> {
    if transaction.required_threads.is_empty() {
        vec![&transaction.thread]
    } else {
        transaction.required_threads.iter().collect()
    }
}

pub fn validate_required_threads(transaction: &Transaction) -> Result<(), ClewError> {
    let threads = transaction_threads(transaction);
    if !transaction.required_threads.is_empty() {
        let primary_hash = canonical::hash(&transaction.thread).map_err(internal)?;
        let required_primary_hash = canonical::hash(threads[0]).map_err(internal)?;
        if primary_hash != required_primary_hash {
            return Err(ClewError::new(
                ErrorCode::InvalidInput,
                "requiredThreads must begin with the exact primary thread",
            ));
        }
    }
    let mut ids = BTreeSet::new();
    for thread in threads {
        if thread.snapshot.base_revision != transaction.base_revision
            || thread.snapshot.project_model_hash != transaction.project_model_hash
            || thread.snapshot.compilation != transaction.thread.snapshot.compilation
            || thread.seed.get("symbol").and_then(Value::as_str).is_none()
        {
            return Err(ClewError::new(
                ErrorCode::InvalidInput,
                "required transaction threads must share revision, project model, and compilation and have a symbol seed",
            ));
        }
        if !ids.insert(&thread.thread_id) {
            return Err(ClewError::new(
                ErrorCode::InvalidInput,
                "required transaction thread IDs must be unique",
            ));
        }
    }
    Ok(())
}

pub(crate) fn validate_worktree(
    worktree: &Path,
    build_system: BuildSystem,
    build_launcher: &str,
    compile_task: &str,
    tests: &[String],
) -> Result<(u64, u64), ClewError> {
    validate_worktree_with_options(
        worktree,
        build_system,
        build_launcher,
        compile_task,
        tests,
        false,
    )
}

pub(crate) fn validate_worktree_fresh(
    worktree: &Path,
    build_system: BuildSystem,
    build_launcher: &str,
    compile_task: &str,
    tests: &[String],
) -> Result<(u64, u64), ClewError> {
    validate_worktree_with_options(
        worktree,
        build_system,
        build_launcher,
        compile_task,
        tests,
        true,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TestLifecycleOutcome {
    pub success: bool,
    pub duration_ms: u64,
}

/// Runs an authority-selected exact test lifecycle while preserving the
/// process result for differential validation. A failing selected test is an
/// observation here, not yet a `TestFailed` error; the authority must pair it
/// with the current-run structured report before accepting it as evidence.
pub(crate) fn run_test_lifecycle_fresh(
    worktree: &Path,
    build_system: BuildSystem,
    build_launcher: &str,
    tests: &[String],
) -> Result<TestLifecycleOutcome, ClewError> {
    if tests.is_empty() {
        return Err(ClewError::new(
            ErrorCode::InvalidInput,
            "exact differential test lifecycle is empty",
        ));
    }
    let started = std::time::Instant::now();
    let mut test = build_command(worktree, build_system, build_launcher)?;
    test.args(tests);
    match build_system {
        BuildSystem::Gradle => {
            test.args(["--no-daemon", "--quiet", "--rerun-tasks"]);
        }
        BuildSystem::Maven => {
            test.arg("-q");
        }
    }
    let output = test
        .current_dir(worktree)
        .output()
        .map_err(|error| build_start_error(build_launcher, error))?;
    Ok(TestLifecycleOutcome {
        success: output.status.success(),
        duration_ms: started.elapsed().as_millis() as u64,
    })
}

fn validate_worktree_with_options(
    worktree: &Path,
    build_system: BuildSystem,
    build_launcher: &str,
    compile_task: &str,
    tests: &[String],
    force_tests: bool,
) -> Result<(u64, u64), ClewError> {
    if build_system == BuildSystem::Maven && !tests.is_empty() {
        // Maven's test lifecycle already includes main/test compilation. A
        // separate `compile` invocation repeats dependency resolution and
        // compilation without increasing validation coverage.
        let test_started = std::time::Instant::now();
        let mut test = build_command(worktree, build_system, build_launcher)?;
        test.args(tests).arg("-q");
        let output = test
            .current_dir(worktree)
            .output()
            .map_err(|error| build_start_error(build_launcher, error))?;
        let test_duration_ms = test_started.elapsed().as_millis() as u64;
        if output.status.success() {
            return Ok((0, test_duration_ms));
        }
        log_output(&output);
        let mut error = ClewError::new(
            ErrorCode::TestFailed,
            format!(
                "candidate worktree {build_system:?} test lifecycle {} failed",
                tests.join(", ")
            ),
        );
        error
            .evidence
            .push("buildCompileCoveredByTestLifecycle=true".into());
        error
            .evidence
            .push(format!("buildTestDurationMs={test_duration_ms}"));
        append_test_failure_evidence(&mut error, worktree, build_system, tests);
        return Err(error);
    }
    let compile_started = std::time::Instant::now();
    let mut compile = build_command(worktree, build_system, build_launcher)?;
    match build_system {
        BuildSystem::Gradle => {
            compile.arg(compile_task).arg("--no-daemon").arg("--quiet");
        }
        BuildSystem::Maven => {
            compile.args(["-q", "-DskipTests", compile_task]);
        }
    }
    let output = compile
        .current_dir(worktree)
        .output()
        .map_err(|error| build_start_error(build_launcher, error))?;
    let compile_duration_ms = compile_started.elapsed().as_millis() as u64;
    if !output.status.success() {
        log_output(&output);
        let mut error = ClewError::new(
            ErrorCode::CompileFailed,
            format!("candidate worktree {build_system:?} compile task {compile_task} failed"),
        );
        error
            .evidence
            .push(format!("buildCompileDurationMs={compile_duration_ms}"));
        return Err(error);
    }
    if tests.is_empty() {
        return Ok((compile_duration_ms, 0));
    }
    let test_started = std::time::Instant::now();
    let mut test = build_command(worktree, build_system, build_launcher)?;
    test.args(tests);
    match build_system {
        BuildSystem::Gradle => {
            test.arg("--no-daemon").arg("--quiet");
            if force_tests {
                test.arg("--rerun-tasks");
            }
        }
        BuildSystem::Maven => {
            test.arg("-q");
        }
    }
    let output = test
        .current_dir(worktree)
        .output()
        .map_err(|error| build_start_error(build_launcher, error))?;
    let test_duration_ms = test_started.elapsed().as_millis() as u64;
    if output.status.success() {
        Ok((compile_duration_ms, test_duration_ms))
    } else {
        log_output(&output);
        let mut error = ClewError::new(
            ErrorCode::TestFailed,
            format!(
                "candidate worktree {build_system:?} test tasks {} failed",
                tests.join(", ")
            ),
        );
        error
            .evidence
            .push(format!("buildCompileDurationMs={compile_duration_ms}"));
        error
            .evidence
            .push(format!("buildTestDurationMs={test_duration_ms}"));
        append_test_failure_evidence(&mut error, worktree, build_system, tests);
        Err(error)
    }
}

fn append_test_failure_evidence(
    error: &mut ClewError,
    worktree: &Path,
    build_system: BuildSystem,
    tests: &[String],
) {
    for failure in bounded_test_failure_evidence(worktree, build_system, tests) {
        error.evidence.push(failure);
    }
}

fn bounded_test_failure_evidence(
    worktree: &Path,
    build_system: BuildSystem,
    tests: &[String],
) -> Vec<String> {
    let mut roots = BTreeSet::new();
    match build_system {
        BuildSystem::Gradle => {
            for argument in tests {
                if argument.starts_with('-') {
                    continue;
                }
                let components = argument
                    .trim_start_matches(':')
                    .split(':')
                    .filter(|component| !component.is_empty())
                    .collect::<Vec<_>>();
                let Some(task) = components.last() else {
                    continue;
                };
                if !task.to_ascii_lowercase().contains("test")
                    || task.eq_ignore_ascii_case("cleanTest")
                {
                    continue;
                }
                let mut root = worktree.to_path_buf();
                for component in &components[..components.len().saturating_sub(1)] {
                    root.push(component);
                }
                roots.insert(root.join("build/test-results").join(task));
            }
        }
        BuildSystem::Maven => {
            roots.insert(worktree.join("target/surefire-reports"));
            roots.insert(worktree.join("target/failsafe-reports"));
        }
    }
    let canonical_worktree = match worktree.canonicalize() {
        Ok(path) => path,
        Err(_) => return Vec::new(),
    };
    let mut failures = Vec::new();
    for root in roots {
        let Ok(metadata) = std::fs::symlink_metadata(&root) else {
            continue;
        };
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            continue;
        }
        let Ok(canonical_root) = root.canonicalize() else {
            continue;
        };
        if !canonical_root.starts_with(&canonical_worktree) {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&canonical_root) else {
            continue;
        };
        for entry in entries.flatten() {
            if failures.len() >= 4 {
                break;
            }
            let path = entry.path();
            let Ok(metadata) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if metadata.file_type().is_symlink()
                || !metadata.is_file()
                || metadata.len() > 2 * 1024 * 1024
                || path.extension().and_then(|value| value.to_str()) != Some("xml")
            {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            failures.extend(junit_failure_evidence(&text, &canonical_worktree));
            failures.truncate(4);
        }
    }
    failures
}

fn junit_failure_evidence(text: &str, worktree: &Path) -> Vec<String> {
    let mut evidence = Vec::new();
    let mut cursor = 0usize;
    while evidence.len() < 4 {
        let Some(relative_start) = text[cursor..].find("<testcase") else {
            break;
        };
        let start = cursor + relative_start;
        let Some(relative_tag_end) = text[start..].find('>') else {
            break;
        };
        let tag_end = start + relative_tag_end + 1;
        let tag = &text[start..tag_end];
        let Some(relative_close) = text[tag_end..].find("</testcase>") else {
            cursor = tag_end;
            continue;
        };
        let close = tag_end + relative_close;
        let body = &text[tag_end..close];
        let failure_start = body.find("<failure").or_else(|| body.find("<error"));
        if let Some(failure_start) = failure_start {
            let failure = &body[failure_start..];
            if let Some(relative_failure_end) = failure.find('>') {
                let failure_tag = &failure[..=relative_failure_end];
                let class_name = xml_attribute_value(tag, "classname").unwrap_or("unknown");
                let test_name = xml_attribute_value(tag, "name").unwrap_or("unknown");
                let failure_type = xml_attribute_value(failure_tag, "type").unwrap_or("unknown");
                let message = xml_attribute_value(failure_tag, "message").unwrap_or("no message");
                let summary = format!(
                    "testFailure={}#{}:{}:{}",
                    class_name, test_name, failure_type, message
                );
                evidence.push(sanitize_test_failure_summary(&summary, worktree));
            }
        }
        cursor = close + "</testcase>".len();
    }
    evidence
}

fn xml_attribute_value<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let needle = format!("{name}=\"");
    let start = tag.find(&needle)? + needle.len();
    let end = tag[start..].find('"')? + start;
    Some(&tag[start..end])
}

fn sanitize_test_failure_summary(summary: &str, worktree: &Path) -> String {
    let worktree = worktree.to_string_lossy();
    let compact = summary
        .replace(worktree.as_ref(), "<worktree>")
        .replace("&quot;", "\"")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    compact.chars().take(768).collect()
}

fn checked_out_target_is_clean(repo: &Path, target_ref: &str, current: &str) -> bool {
    git_output(repo, &["symbolic-ref", "-q", "HEAD"]).is_ok_and(|head| head == target_ref)
        && Command::new("git")
            .args(["diff-index", "--quiet", current, "--"])
            .current_dir(repo)
            .status()
            .is_ok_and(|status| status.success())
}

fn build_command(
    worktree: &Path,
    build_system: BuildSystem,
    build_launcher: &str,
) -> Result<Command, ClewError> {
    let launcher = match (build_system, build_launcher) {
        (BuildSystem::Gradle, "./gradlew") => worktree.join("gradlew"),
        (BuildSystem::Maven, "./mvnw") => worktree.join("mvnw"),
        (BuildSystem::Maven, "mvn") => PathBuf::from("mvn"),
        _ => {
            return Err(ClewError::new(
                ErrorCode::UnsupportedProjectConfiguration,
                format!("unsupported {build_system:?} build launcher policy {build_launcher}"),
            ));
        }
    };
    if launcher.is_absolute() && !launcher.is_file() {
        return Err(ClewError::new(
            ErrorCode::UnsupportedProjectConfiguration,
            format!("stored build launcher is missing: {}", launcher.display()),
        ));
    }
    Ok(Command::new(launcher))
}

fn build_start_error(build_launcher: &str, error: std::io::Error) -> ClewError {
    ClewError::new(
        ErrorCode::UnsupportedProjectConfiguration,
        format!("build launcher {build_launcher} could not start: {error}"),
    )
}

/// Materialize the ignored repository-owned dependency state alongside a
/// detached candidate worktree. A linked worktree intentionally contains only
/// tracked files, while the Kotlin worker's legacy development contour reads
/// Gradle/Maven state below the repository root. Sharing it by symlink or
/// hardlink would let candidate validation mutate the caller's state, so we
/// require an isolated copy-on-write clone instead.
fn prepare_candidate_repository_state(
    repo: &Path,
    worktree: &Path,
    build_system: BuildSystem,
) -> Result<(), ClewError> {
    let relative = match build_system {
        BuildSystem::Gradle => Path::new(".gradle"),
        BuildSystem::Maven => Path::new(".semantic-thread/maven-repository"),
    };
    let source = repo.join(relative);
    if !source.exists() {
        return Ok(());
    }
    let metadata = std::fs::symlink_metadata(&source).map_err(io_error)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(ClewError::new(
            ErrorCode::UnsupportedProjectConfiguration,
            format!(
                "repository-owned build state is not a real directory: {}",
                relative.display()
            ),
        ));
    }
    let ignored = Command::new("git")
        .args(["check-ignore", "--quiet", "--"])
        .arg(relative)
        .current_dir(repo)
        .status()
        .map_err(io_error)?
        .success();
    if !ignored {
        return Err(ClewError::new(
            ErrorCode::UnsupportedProjectConfiguration,
            format!(
                "repository-owned build state must be ignored before candidate validation: {}",
                relative.display()
            ),
        ));
    }
    let destination = worktree.join(relative);
    if destination.exists() {
        return Err(ClewError::new(
            ErrorCode::UnsupportedProjectConfiguration,
            format!(
                "candidate build-state destination already exists: {}",
                relative.display()
            ),
        ));
    }
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent).map_err(io_error)?;
    }
    clone_directory_copy_on_write(&source, &destination)?;
    let copied = std::fs::symlink_metadata(&destination).map_err(io_error)?;
    if copied.file_type().is_symlink() || !copied.is_dir() {
        return Err(ClewError::new(
            ErrorCode::UnsupportedProjectConfiguration,
            "candidate build-state snapshot is not an isolated directory",
        ));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn clone_directory_copy_on_write(source: &Path, destination: &Path) -> Result<(), ClewError> {
    let output = Command::new("/bin/cp")
        .arg("-cR")
        .arg(source)
        .arg(destination)
        .output()
        .map_err(io_error)?;
    if output.status.success() {
        return Ok(());
    }
    Err(ClewError::new(
        ErrorCode::UnsupportedProjectConfiguration,
        format!(
            "candidate build-state copy-on-write snapshot failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ),
    ))
}

#[cfg(target_os = "linux")]
fn clone_directory_copy_on_write(source: &Path, destination: &Path) -> Result<(), ClewError> {
    let output = Command::new("cp")
        .args(["--archive", "--reflink=always"])
        .arg(source)
        .arg(destination)
        .output()
        .map_err(io_error)?;
    if output.status.success() {
        return Ok(());
    }
    Err(ClewError::new(
        ErrorCode::UnsupportedProjectConfiguration,
        format!(
            "candidate build-state reflink snapshot failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ),
    ))
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn clone_directory_copy_on_write(_source: &Path, _destination: &Path) -> Result<(), ClewError> {
    Err(ClewError::new(
        ErrorCode::UnsupportedProjectConfiguration,
        "candidate build-state isolation requires copy-on-write filesystem support",
    ))
}

pub struct Ledger {
    connection: Connection,
    repo: PathBuf,
}
impl Ledger {
    pub fn open(repo: &Path) -> Result<Self, ClewError> {
        let dir = repo.join(".semantic-thread");
        std::fs::create_dir_all(&dir).map_err(io_error)?;
        let connection = Connection::open(dir.join("ledger.sqlite3")).map_err(db_error)?;
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .map_err(db_error)?;
        connection.execute_batch("CREATE TABLE IF NOT EXISTS events(sequence INTEGER PRIMARY KEY AUTOINCREMENT, tx_id TEXT NOT NULL, status TEXT NOT NULL, timestamp TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP, record_json BLOB NOT NULL, evidence TEXT NOT NULL);") .map_err(db_error)?;
        Ok(Self {
            connection,
            repo: repo.to_path_buf(),
        })
    }
    pub fn append(&self, tx: &Transaction, evidence: &str) -> Result<(), ClewError> {
        self.connection
            .execute(
                "INSERT INTO events(tx_id,status,record_json,evidence) VALUES(?1,?2,?3,?4)",
                params![
                    tx.tx_id,
                    tx.status,
                    canonical::bytes(tx).map_err(internal)?,
                    evidence
                ],
            )
            .map_err(db_error)?;
        Ok(())
    }
    pub fn inspect(&self, id: &str) -> Result<Value, ClewError> {
        let latest: Option<(String, Vec<u8>)> = self.connection.query_row(
            "SELECT status,record_json FROM events WHERE tx_id=?1 ORDER BY sequence DESC LIMIT 1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional().map_err(db_error)?;
        let (mut status, record) = latest.ok_or_else(|| {
            ClewError::new(
                ErrorCode::InvalidInput,
                format!("transaction not found: {id}"),
            )
        })?;
        let mut transaction: Transaction = serde_json::from_slice(&record).map_err(|error| {
            ClewError::new(ErrorCode::TransactionRecoveryRequired, error.to_string())
        })?;
        let terminal = matches!(
            status.as_str(),
            "COMMITTED" | "CONFLICTED" | "STALE_REQUIRES_RESLICE" | "VALIDATION_FAILED" | "ABORTED"
        );
        let mut action = "NONE";
        if !terminal {
            let target_revision = transaction
                .target_ref
                .as_deref()
                .map(|target_ref| git_output(&self.repo, &["rev-parse", target_ref]))
                .transpose()?;
            let reachable_commit = target_revision
                .as_deref()
                .map(|revision| find_transaction_commit(&self.repo, revision, id))
                .transpose()?
                .flatten();
            if let Some(commit) = reachable_commit {
                let compilation = &transaction.thread.snapshot.compilation;
                let index = RepositoryIndex::open_compilation(&self.repo, Some(compilation))?;
                let current_target = target_revision
                    .as_deref()
                    .expect("reachable commit has target");
                if index.published_revision()?.as_deref() != Some(current_target) {
                    drop(index);
                    let mut worker = WorkerClient::start(&workspace_root())?;
                    let publication = publish_index_for_revision(
                        &self.repo,
                        current_target,
                        compilation,
                        transaction.thread.snapshot.build_system,
                        &mut worker,
                    );
                    let _ = worker.shutdown();
                    let (index_snapshot, invalidations) = publication?;
                    transaction.validation_evidence.push(json!({
                        "kind":"INDEX_RECOVERY",
                        "finalIndexSnapshot":index_snapshot,
                        "appliedInvalidations":invalidations
                    }));
                }
                transaction.status = "COMMITTED".into();
                transaction.final_commit = Some(commit);
                self.append(
                    &transaction,
                    "recovered committed status from reachable Git trailer",
                )?;
                status = "COMMITTED".into();
                action = "RECOVERED_COMMITTED_FROM_TRAILER";
            } else if status == "COMMITTING" {
                match recover_candidate_commit(&self.repo, &transaction)? {
                    CandidateRecovery::Published(commit) => {
                        transaction.status = "COMMITTED".into();
                        transaction.final_commit = Some(commit);
                        self.append(
                            &transaction,
                            "recovered candidate commit with compare-and-swap",
                        )?;
                        status = "COMMITTED".into();
                        action = "RECOVERED_COMMITTED_CANDIDATE_CAS";
                    }
                    CandidateRecovery::Conflicted(reason) => {
                        transaction.status = "CONFLICTED".into();
                        self.append(&transaction, &reason)?;
                        status = "CONFLICTED".into();
                        action = "RECOVERED_CONFLICTED_REF_MOVED";
                    }
                    CandidateRecovery::Aborted(reason) => {
                        transaction.status = "ABORTED".into();
                        self.append(&transaction, &reason)?;
                        status = "ABORTED".into();
                        action = "RECOVERED_ABORTED_CANDIDATE_MISSING";
                    }
                }
            } else if matches!(
                status.as_str(),
                "CREATED" | "SLICED" | "EDIT_PREVIEWED" | "VALIDATING" | "VALIDATED" | "REBASING"
            ) {
                transaction.status = "ABORTED".into();
                self.append(
                    &transaction,
                    "recovered unfinished pre-publication transaction as aborted",
                )?;
                status = "ABORTED".into();
                action = "RECOVERED_ABORTED_NO_REF_CHANGE";
            } else {
                transaction.status = "ABORTED".into();
                self.append(
                    &transaction,
                    "recovered unknown unfinished state as aborted",
                )?;
                status = "ABORTED".into();
                action = "RECOVERED_ABORTED_UNKNOWN_STATE";
            }
        }
        let mut statement=self.connection.prepare("SELECT sequence,status,timestamp,evidence FROM events WHERE tx_id=?1 ORDER BY sequence").map_err(db_error)?;
        let rows=statement.query_map([id],|r|Ok(json!({"sequence":r.get::<_,i64>(0)?,"status":r.get::<_,String>(1)?,"timestamp":r.get::<_,String>(2)?,"evidence":r.get::<_,String>(3)?}))).map_err(db_error)?.collect::<Result<Vec<_>,_>>().map_err(db_error)?;
        Ok(
            json!({"schema":"semantic-ledger/0.1","transactionId":id,"events":rows,"reconciledStatus":status,"recoveryAction":action,"recoverable":true}),
        )
    }
}

enum CandidateRecovery {
    Published(String),
    Conflicted(String),
    Aborted(String),
}

fn recover_candidate_commit(
    repo: &Path,
    transaction: &Transaction,
) -> Result<CandidateRecovery, ClewError> {
    let Some(candidate) = transaction.candidate_commit.as_deref() else {
        return Ok(CandidateRecovery::Aborted(
            "COMMITTING record has no candidate commit; no ref was changed".into(),
        ));
    };
    let Some(target_ref) = transaction.target_ref.as_deref() else {
        return Ok(CandidateRecovery::Aborted(
            "COMMITTING record has no target ref; candidate left unpublished".into(),
        ));
    };
    let message = match git_output(repo, &["show", "-s", "--format=%B", candidate]) {
        Ok(message) => message,
        Err(_) => {
            return Ok(CandidateRecovery::Aborted(
                "candidate commit object is unavailable; no ref was changed".into(),
            ));
        }
    };
    let edit_hash = canonical::hash(&transaction.edit).map_err(internal)?;
    if !message
        .lines()
        .any(|line| line.trim() == format!("Semantic-Transaction-Id: {}", transaction.tx_id))
        || !message
            .lines()
            .any(|line| line.trim() == format!("Semantic-Edit-Hash: {edit_hash}"))
    {
        return Ok(CandidateRecovery::Aborted(
            "candidate trailers do not match ledger transaction".into(),
        ));
    }
    let parent = git_output(repo, &["rev-parse", &format!("{candidate}^")])?;
    let current = git_output(repo, &["rev-parse", target_ref])?;
    if current != parent {
        return Ok(CandidateRecovery::Conflicted(format!(
            "target ref moved from candidate parent {parent} to {current} before recovery"
        )));
    }
    let compilation = &transaction.thread.snapshot.compilation;
    let mut worker = WorkerClient::start(&workspace_root())?;
    let staged = stage_index_for_revision(
        repo,
        candidate,
        compilation,
        transaction.thread.snapshot.build_system,
        &mut worker,
    );
    let _ = worker.shutdown();
    let staged = staged?;
    git(repo, &["update-ref", target_ref, candidate, &current])?;
    if let Err(publication_error) = staged.publish() {
        if git(repo, &["update-ref", target_ref, &current, candidate]).is_ok() {
            return Err(index_recovery_error(ClewError::new(
                ErrorCode::TransactionRecoveryRequired,
                format!(
                    "candidate index publication failed; target ref was rolled back: {}",
                    publication_error.message
                ),
            )));
        }
        return Err(index_recovery_error(ClewError::new(
            ErrorCode::TransactionRecoveryRequired,
            format!(
                "candidate index publication failed and target ref rollback failed: {}",
                publication_error.message
            ),
        )));
    }
    Ok(CandidateRecovery::Published(candidate.into()))
}

fn find_transaction_commit(
    repo: &Path,
    revision: &str,
    id: &str,
) -> Result<Option<String>, ClewError> {
    let output = Command::new("git")
        .args(["log", revision, "--format=%H%x1f%B%x1e"])
        .current_dir(repo)
        .output()
        .map_err(io_error)?;
    if !output.status.success() {
        return Err(ClewError::new(
            ErrorCode::TransactionRecoveryRequired,
            "cannot scan Git history for transaction trailers",
        ));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(text.split('\u{1e}').find_map(|record| {
        let (commit, message) = record.trim().split_once('\u{1f}')?;
        message
            .lines()
            .any(|line| line.trim() == format!("Semantic-Transaction-Id: {id}"))
            .then(|| commit.to_owned())
    }))
}

fn find_matching_transaction_commit(
    repo: &Path,
    revision: &str,
    id: &str,
    edit_hash: &str,
) -> Result<Option<String>, ClewError> {
    let output = Command::new("git")
        .args(["log", revision, "--format=%H%x1f%B%x1e"])
        .current_dir(repo)
        .output()
        .map_err(io_error)?;
    if !output.status.success() {
        return Err(ClewError::new(
            ErrorCode::Internal,
            format!("cannot scan target history {revision} for transaction trailers"),
        ));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    for record in text.split('\u{1e}') {
        let Some((commit, message)) = record.trim().split_once('\u{1f}') else {
            continue;
        };
        if !message
            .lines()
            .any(|line| line.trim() == format!("Semantic-Transaction-Id: {id}"))
        {
            continue;
        }
        let recorded_hash = message.lines().find_map(|line| {
            line.trim()
                .strip_prefix("Semantic-Edit-Hash: ")
                .map(str::to_owned)
        });
        if recorded_hash.as_deref() != Some(edit_hash) {
            return Err(ClewError::new(
                ErrorCode::InvalidInput,
                format!("transaction id {id} is already associated with a different edit"),
            ));
        }
        return Ok(Some(commit.to_owned()));
    }
    Ok(None)
}
pub fn ledger(repo: &Path) -> Result<Ledger, ClewError> {
    Ledger::open(repo)
}

fn unified_diff(file: &str, old: &str, new: &str) -> String {
    if old == new {
        return String::new();
    }
    TextDiff::from_lines(old, new)
        .unified_diff()
        .context_radius(3)
        .header(&format!("a/{file}"), &format!("b/{file}"))
        .to_string()
}

fn safe_join(root: &Path, relative: &str) -> Result<PathBuf, ClewError> {
    let path = root.join(relative);
    if relative.starts_with('/') || relative.split('/').any(|p| p == "..") {
        Err(ClewError::new(
            ErrorCode::InvalidInput,
            "path escapes repository",
        ))
    } else {
        Ok(path)
    }
}
fn git(repo: &Path, args: &[&str]) -> Result<(), ClewError> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .map_err(io_error)?;
    if output.status.success() {
        Ok(())
    } else {
        log_output(&output);
        Err(ClewError::new(
            ErrorCode::Internal,
            format!("git {} failed", args.join(" ")),
        ))
    }
}
fn log_output(output: &std::process::Output) {
    if !output.stdout.is_empty() {
        eprint!("{}", String::from_utf8_lossy(&output.stdout));
    }
    if !output.stderr.is_empty() {
        eprint!("{}", String::from_utf8_lossy(&output.stderr));
    }
}
fn git_output(repo: &Path, args: &[&str]) -> Result<String, ClewError> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .map_err(io_error)?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    } else {
        Err(ClewError::new(
            ErrorCode::InvalidInput,
            String::from_utf8_lossy(&output.stderr).trim(),
        ))
    }
}
fn io_error(e: std::io::Error) -> ClewError {
    ClewError::new(ErrorCode::Internal, e.to_string())
}
fn db_error(e: rusqlite::Error) -> ClewError {
    ClewError::new(ErrorCode::Internal, e.to_string())
}
fn internal(e: anyhow::Error) -> ClewError {
    ClewError::new(ErrorCode::Internal, e.to_string())
}

#[cfg(test)]
mod tests {
    use super::{
        BuildSystem, apply_edit_target_transport, bounded_test_failure_evidence,
        canonicalize_owner_symbol_query, git_output, prepare_candidate_repository_state, preview,
        preview_replace_model_input, project_model_transition_evidence, validate_worktree,
    };
    use crate::canonical;
    use crate::error::ErrorCode;
    use crate::model::{
        Completeness, CompletenessStatus, EditIr, EditOperation, ExpectedWriteFact, Replacement,
        SemanticOperation, SlicePolicy, Snapshot, ThreadIr, WriteFact,
    };
    use crate::worker::{WorkerClient, workspace_root};
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::path::Path;
    use std::process::Command;

    fn owner_identity(parameter_types: &[&str], receiver_types: &[&str]) -> String {
        json!({
            "module":":workers:kotlin21",
            "sourceSet":"main",
            "package":"dev.semanticthread.worker",
            "containingDeclarations":["Worker"],
            "declarationName":"index",
            "declarationKind":"FUNCTION",
            "typeParameterArity":0,
            "receiverTypes":receiver_types,
            "contextReceiverTypes":[],
            "parameterTypes":parameter_types,
            "returnType":"kotlinx/serialization/json/JsonObject",
            "suspendFlag":false,
            "jvmDescriptor":"(Ljava/nio/file/Path;)Ljava/lang/Object;",
        })
        .to_string()
    }

    #[test]
    fn apply_edit_owner_query_canonicalizes_qualified_types_inside_generics() {
        let original = owner_identity(
            &[
                "java/nio/file/Path",
                "kotlin/String?",
                "kotlin/Boolean",
                "kotlin.collections.List<kotlin.String>",
            ],
            &["kotlin/collections/Map<kotlin.String,java.nio.file.Path?>"],
        );

        let query: serde_json::Value = serde_json::from_str(
            &canonicalize_owner_symbol_query(&original).expect("complete compiler identity"),
        )
        .unwrap();

        assert_eq!(
            query["parameterTypes"],
            json!(["Path", "String?", "Boolean", "List<String>"])
        );
        assert_eq!(query["receiverTypes"], json!(["Map<String,Path?>"]));
        assert_eq!(query["declarationName"], "index");
    }

    #[test]
    fn apply_edit_owner_query_keeps_distinct_generic_leaf_names_distinct() {
        let first = canonicalize_owner_symbol_query(&owner_identity(
            &["kotlin/collections/List<com/acme/First>"],
            &[],
        ))
        .unwrap();
        let second = canonicalize_owner_symbol_query(&owner_identity(
            &["kotlin/collections/List<com/acme/Second>"],
            &[],
        ))
        .unwrap();
        let first: serde_json::Value = serde_json::from_str(&first).unwrap();
        let second: serde_json::Value = serde_json::from_str(&second).unwrap();

        assert_eq!(first["parameterTypes"], json!(["List<First>"]));
        assert_eq!(second["parameterTypes"], json!(["List<Second>"]));
        assert_ne!(first["parameterTypes"], second["parameterTypes"]);
    }

    #[test]
    fn malformed_apply_edit_owner_query_is_passed_through_without_guessing() {
        let non_json = "dev.semanticthread.worker.Worker.index";
        assert!(canonicalize_owner_symbol_query(non_json).is_none());
        let malformed_type = owner_identity(&["kotlin/collections/List<kotlin/String"], &[]);
        assert!(canonicalize_owner_symbol_query(&malformed_type).is_none());

        let target = json!({
            "ownerSymbolId":malformed_type,
            "exactTextHash":"sha256:original-authority",
            "syntaxKind":"KtNamedFunction",
        });
        let transport = apply_edit_target_transport(target.as_object().unwrap());
        assert_eq!(transport["ownerSymbolId"], target["ownerSymbolId"]);
    }

    #[test]
    fn apply_edit_transport_keeps_original_authority_hash_and_target_unchanged() {
        let owner = owner_identity(&["kotlin/collections/List<kotlin/String>"], &[]);
        let target = json!({
            "ownerSymbolId":owner,
            "exactTextHash":"sha256:original-authority",
            "syntaxKind":"KtNamedFunction",
            "rangeHint":[72022,79757],
        });
        let original_target = target.clone();

        let transport = apply_edit_target_transport(target.as_object().unwrap());

        assert_eq!(transport["exactTextHash"], "sha256:original-authority");
        assert_eq!(transport["syntaxKind"], "KtNamedFunction");
        assert_ne!(transport["ownerSymbolId"], target["ownerSymbolId"]);
        assert_eq!(target, original_target);
    }

    fn model_project(path: &str, hash: &str, project_hash: &str) -> serde_json::Value {
        let manifest = json!({
            "schema":"kotlin-semantic-input-manifest/0.1",
            "orderedCompileClasspath":[],
            "modelInputs":[{"path":path,"hash":hash}],
        });
        json!({
            "schema":"semantic-project/0.1",
            "projectModelHash":project_hash,
            "semanticInputManifestHash":canonical::hash(&manifest).unwrap(),
            "semanticInputManifest":manifest,
        })
    }

    #[test]
    fn model_input_preview_is_a_whole_file_cas_and_detects_concurrent_change() {
        let temporary = tempfile::tempdir().unwrap();
        let repo = temporary.path();
        let path = "workers/kotlin21/build.gradle.kts";
        let original = "plugins {}\n";
        std::fs::create_dir_all(repo.join("workers/kotlin21")).unwrap();
        std::fs::write(repo.join(path), original).unwrap();
        assert!(
            Command::new("git")
                .args(["init", "--quiet", "--initial-branch=main"])
                .current_dir(repo)
                .status()
                .unwrap()
                .success()
        );
        assert!(
            Command::new("git")
                .args(["add", "--", path])
                .current_dir(repo)
                .status()
                .unwrap()
                .success()
        );
        let before_hash = canonical::hash_bytes(original.as_bytes());
        let project = model_project(path, &before_hash, "sha256:before-project");
        let manifest_hash = project["semanticInputManifestHash"].clone();
        let thread = ThreadIr {
            schema: "semantic-thread/0.2".into(),
            thread_id: "thread:model-input".into(),
            snapshot: Snapshot {
                project_model_hash: "sha256:before-project".into(),
                ..Snapshot::default()
            },
            seed: json!({}),
            policy: SlicePolicy::default(),
            completeness: Completeness {
                status: CompletenessStatus::CompleteSupportedSubset,
                boundaries: vec![],
            },
            nodes: vec![],
            edges: vec![],
            editable_units: vec![],
            external_summaries: vec![],
            read_set: vec![],
            validation_plan: vec![],
        };
        let operation = EditOperation {
            op_id: "replace-model".into(),
            kind: "REPLACE_MODEL_INPUT".into(),
            target: json!({
                "fileId":path,
                "exactTextHash":before_hash,
                "syntaxKind":"MODEL_INPUT_FILE",
                "semanticInputManifestHash":manifest_hash,
            }),
            replacement: Replacement {
                kotlin: "plugins { kotlin(\"jvm\") }\n".into(),
            },
            semantic_operation: None,
            preconditions: BTreeMap::new(),
            postconditions: BTreeMap::new(),
        };
        let mut candidates = BTreeMap::new();
        let mut writes = Vec::new();
        let mut expected = Vec::<ExpectedWriteFact>::new();
        let mut windows = Vec::new();

        preview_replace_model_input(
            repo,
            &thread,
            &operation,
            operation.target.as_object().unwrap(),
            path,
            &project,
            &mut candidates,
            &mut writes,
            &mut expected,
            &mut windows,
        )
        .unwrap();

        assert_eq!(candidates[path], "plugins { kotlin(\"jvm\") }\n");
        assert_eq!(writes[0].kind, "MODEL_INPUT");
        assert_eq!(writes[0].before_hash, before_hash);
        assert_eq!(expected[0].kind, "MODEL_INPUT");

        std::fs::write(repo.join(path), "// concurrent\nplugins {}\n").unwrap();
        let error = preview_replace_model_input(
            repo,
            &thread,
            &operation,
            operation.target.as_object().unwrap(),
            path,
            &project,
            &mut BTreeMap::new(),
            &mut Vec::new(),
            &mut Vec::new(),
            &mut Vec::new(),
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::WwConflict);
    }

    #[test]
    fn only_model_input_write_authorizes_and_records_project_model_drift() {
        let path = "workers/kotlin21/build.gradle.kts";
        let before_hash = canonical::hash_bytes(b"plugins {}\n");
        let after_hash = canonical::hash_bytes(b"plugins { kotlin(\"jvm\") }\n");
        let before = model_project(path, &before_hash, "sha256:before-project");
        let after = model_project(path, &after_hash, "sha256:after-project");

        let ordinary_error = project_model_transition_evidence(&before, &after, &[]).unwrap_err();
        assert_eq!(ordinary_error.code, ErrorCode::ProjectModelChanged);

        let wrong_kind = [WriteFact {
            kind: "FILE".into(),
            key: path.into(),
            before_hash: before_hash.clone(),
            after_hash: after_hash.clone(),
        }];
        assert!(project_model_transition_evidence(&before, &after, &wrong_kind).is_err());

        let authorized = [WriteFact {
            kind: "MODEL_INPUT".into(),
            key: path.into(),
            before_hash,
            after_hash,
        }];
        let evidence = project_model_transition_evidence(&before, &after, &authorized)
            .unwrap()
            .unwrap();
        assert_eq!(evidence["kind"], "PROJECT_MODEL_TRANSITION");
        assert_eq!(evidence["authority"], "REPLACE_MODEL_INPUT");
        assert_eq!(evidence["stableIdentityChanged"], true);
        assert_eq!(evidence["modelInputs"][0]["path"], path);
        assert_eq!(evidence["status"], "PENDING_INDEX_VERIFICATION");
    }

    #[test]
    fn candidate_gets_an_isolated_copy_on_write_build_state() {
        let temporary = tempfile::tempdir().unwrap();
        let repo = temporary.path().join("repo");
        let candidate = temporary.path().join("candidate");
        std::fs::create_dir_all(repo.join(".gradle/caches")).unwrap();
        std::fs::create_dir(&candidate).unwrap();
        std::fs::write(repo.join(".gitignore"), ".gradle/\n").unwrap();
        std::fs::write(repo.join(".gradle/caches/marker"), "source").unwrap();
        assert!(
            Command::new("git")
                .args(["init", "--quiet", "--initial-branch=main"])
                .current_dir(&repo)
                .status()
                .unwrap()
                .success()
        );

        prepare_candidate_repository_state(&repo, &candidate, BuildSystem::Gradle).unwrap();
        std::fs::write(candidate.join(".gradle/caches/marker"), "candidate").unwrap();

        assert_eq!(
            std::fs::read_to_string(repo.join(".gradle/caches/marker")).unwrap(),
            "source"
        );
        assert_eq!(
            std::fs::read_to_string(candidate.join(".gradle/caches/marker")).unwrap(),
            "candidate"
        );
    }

    #[test]
    fn candidate_refuses_to_snapshot_unignored_build_state() {
        let temporary = tempfile::tempdir().unwrap();
        let repo = temporary.path().join("repo");
        let candidate = temporary.path().join("candidate");
        std::fs::create_dir_all(repo.join(".gradle")).unwrap();
        std::fs::create_dir(&candidate).unwrap();
        assert!(
            Command::new("git")
                .args(["init", "--quiet", "--initial-branch=main"])
                .current_dir(&repo)
                .status()
                .unwrap()
                .success()
        );

        let error =
            prepare_candidate_repository_state(&repo, &candidate, BuildSystem::Gradle).unwrap_err();

        assert_eq!(error.code, ErrorCode::UnsupportedProjectConfiguration);
        assert!(!candidate.join(".gradle").exists());
    }

    #[test]
    fn validates_maven_compile_and_tests() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("fixtures/kotlin-maven");

        let result = validate_worktree(
            &fixture,
            BuildSystem::Maven,
            "./mvnw",
            "compile",
            &["test".into()],
        );

        assert!(result.is_ok(), "{result:?}");
    }

    #[test]
    fn selected_gradle_test_failure_is_retained_as_bounded_path_free_evidence() {
        let temporary = tempfile::tempdir().unwrap();
        let worktree = temporary.path().join("repo");
        let reports = worktree.join("workers/kotlin/build/test-results/test");
        std::fs::create_dir_all(&reports).unwrap();
        std::fs::write(
            reports.join("TEST-example.CacheTest.xml"),
            format!(
                r#"<testsuite><testcase name="publishes()" classname="example.CacheTest"><failure message="expected PUBLISHED below {} but was WRITE_FAILED" type="org.opentest4j.AssertionFailedError">stack</failure></testcase></testsuite>"#,
                worktree.display()
            ),
        )
        .unwrap();

        let evidence = bounded_test_failure_evidence(
            &worktree,
            BuildSystem::Gradle,
            &[
                ":workers:kotlin:test".into(),
                "--tests".into(),
                "*CacheTest".into(),
            ],
        );

        assert_eq!(evidence.len(), 1);
        assert!(evidence[0].contains("example.CacheTest#publishes()"));
        assert!(evidence[0].contains("expected PUBLISHED"));
        assert!(evidence[0].contains("<worktree>"));
        assert!(!evidence[0].contains(temporary.path().to_string_lossy().as_ref()));
    }

    #[test]
    fn generic_preview_cannot_apply_an_authority_semantic_operation() {
        let temporary = tempfile::tempdir().unwrap();
        let repo = temporary.path();
        assert!(
            Command::new("git")
                .args(["init", "--quiet", "--initial-branch=main"])
                .current_dir(repo)
                .status()
                .unwrap()
                .success()
        );
        assert!(
            Command::new("git")
                .args([
                    "-c",
                    "user.name=Codeclew Test",
                    "-c",
                    "user.email=codeclew@example.invalid",
                    "commit",
                    "--quiet",
                    "--allow-empty",
                    "-m",
                    "base",
                ])
                .current_dir(repo)
                .status()
                .unwrap()
                .success()
        );
        let head = git_output(repo, &["rev-parse", "HEAD"]).unwrap();
        let thread = ThreadIr {
            schema: "semantic-thread/0.2".into(),
            thread_id: "thread:authority".into(),
            snapshot: Snapshot {
                base_revision: head.clone(),
                ..Snapshot::default()
            },
            seed: json!({}),
            policy: SlicePolicy::default(),
            completeness: Completeness {
                status: CompletenessStatus::CompleteSupportedSubset,
                boundaries: vec![],
            },
            nodes: vec![],
            edges: vec![],
            editable_units: vec![],
            external_summaries: vec![],
            read_set: vec![],
            validation_plan: vec![],
        };
        let edit = EditIr {
            schema: "semantic-edit/0.2".into(),
            thread_id: thread.thread_id.clone(),
            base_revision: head,
            operations: vec![EditOperation {
                op_id: "forged".into(),
                kind: "MAP_EDGE_WITH_CONTEXT".into(),
                target: json!({"fileId":"Runner.kt"}),
                replacement: Replacement {
                    kotlin: String::new(),
                },
                semantic_operation: Some(SemanticOperation::MapEdgeWithContext {
                    workflow_symbol: "com/acme/workflow".into(),
                    context_producer_symbol: "com/acme/context".into(),
                    transformer_symbol: "com/acme/transform".into(),
                    value_parameter_index: 0,
                    collection_type: "kotlin/collections/List<kotlin/Int>".into(),
                    element_type: "kotlin/Int".into(),
                    context_type: "kotlin/Int".into(),
                    placement: "com/acme/workflow#FUNCTION_ENTRY".into(),
                    strategy: "KOTLIN_EAGER_LIST_MAP_WITH_CONTEXT_ONCE".into(),
                }),
                preconditions: BTreeMap::new(),
                postconditions: BTreeMap::new(),
            }],
            expected_write_set: vec![],
        };
        let mut worker = WorkerClient::start(&workspace_root()).unwrap();
        let error = preview(repo, &thread, &edit, &mut worker).unwrap_err();
        assert_eq!(error.code, ErrorCode::PreconditionFailed);
        assert!(error.message.contains("live authority proof receipt"));
        worker.shutdown().unwrap();
    }
}
