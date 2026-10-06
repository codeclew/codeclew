//! Immutable, bounded records used to resume one documentation-agent run.
//!
//! This module stores recovery material only. The parent coordinator owns the
//! state machine and selects a checkpoint by atomically saving its returned
//! `CheckpointRef` in the run report. Unselected checkpoint files are harmless
//! orphans after a crash.

use super::super::{bytes, digest, invalid, io_error};
use super::{
    Usage,
    store::{self, Repository, WriteLock},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::fs;

const CHECKPOINT_REF_SCHEMA: &str = "codeclew-documentation-recovery-checkpoint-ref/1.0";
const CHECKPOINT_FILE_SCHEMA: &str = "codeclew-documentation-recovery-checkpoint/1.0";
const CALL_INPUT_SCHEMA: &str = "codeclew-documentation-agent-input/1.0";
const CALL_RESULT_SCHEMA: &str = "codeclew-documentation-agent-result-record/2.0";
const VALIDATED_REPLY: &str = "VALIDATED_REPLY";
const MODEL_INPUT_SCHEMA: &str = "codeclew-documentation-model-input/1.0";
const RAW_MODEL_RESULT_SCHEMA: &str = "codeclew-documentation-model-wire-result/1.0";
const MAX_RECOVERY_BYTES: usize = store::MAX_RECORD as usize;
#[cfg(test)]
thread_local! {
    static INTERRUPT_MODEL_HEAD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static INTERRUPT_RAW_MODEL_RESULT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
#[cfg(test)]
pub(super) fn interrupt_model_head_once() {
    INTERRUPT_MODEL_HEAD.with(|flag| flag.set(true));
}
#[cfg(test)]
pub(super) fn interrupt_raw_model_result_once() {
    INTERRUPT_RAW_MODEL_RESULT.with(|flag| flag.set(true));
}

/// A pointer is saved in the mutable run report only after its immutable
/// checkpoint file has been written. The relative path is derived from these
/// fields and is never accepted from persisted data.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct CheckpointRef {
    pub schema: String,
    pub run: String,
    pub sequence: u64,
    pub checkpoint_digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CheckpointFile<T> {
    schema: String,
    run: String,
    sequence: u64,
    checkpoint_digest: String,
    checkpoint: T,
}

/// Persist one immutable checkpoint while the caller already owns this
/// repository's write lock. Keep the repository/guard pairing local to the
/// reviewed publication callback; this is not a cross-repository transaction API.
pub(super) fn save_checkpoint_locked<T: Serialize>(
    repo: &Repository,
    guard: &WriteLock,
    run: &str,
    sequence: u64,
    checkpoint: &T,
) -> Result<CheckpointRef, crate::error::ClewError> {
    validate_run_id(run)?;
    let checkpoint_digest = digest(checkpoint)?;
    let file = CheckpointFile {
        schema: CHECKPOINT_FILE_SCHEMA.into(),
        run: run.into(),
        sequence,
        checkpoint_digest: checkpoint_digest.clone(),
        checkpoint,
    };
    let encoded = bounded_bytes(&file, "recovery checkpoint")?;
    let path = checkpoint_path(run, sequence, &checkpoint_digest)?;
    persist_immutable_locked(repo, guard, &path, &encoded, "checkpoint")?;
    Ok(CheckpointRef {
        schema: CHECKPOINT_REF_SCHEMA.into(),
        run: run.into(),
        sequence,
        checkpoint_digest,
    })
}

/// Load and validate the exact checkpoint selected by a report reference.
/// Missing, malformed, oversized, or mismatched state is an error, never a
/// cache miss.
pub(super) fn load_checkpoint<T: DeserializeOwned + Serialize>(
    repo: &Repository,
    reference: &CheckpointRef,
) -> Result<T, crate::error::ClewError> {
    validate_checkpoint_ref(reference)?;
    let path = checkpoint_path(
        &reference.run,
        reference.sequence,
        &reference.checkpoint_digest,
    )?;
    let file: CheckpointFile<T> = read_required(repo, &path, "checkpoint")?;
    if file.schema != CHECKPOINT_FILE_SCHEMA
        || file.run != reference.run
        || file.sequence != reference.sequence
        || file.checkpoint_digest != reference.checkpoint_digest
        || digest(&file.checkpoint)? != reference.checkpoint_digest
    {
        return Err(invalid(
            "RECOVERY_CHECKPOINT_MISMATCH: saved checkpoint binding or digest is invalid",
        ));
    }
    Ok(file.checkpoint)
}

/// Stable identity that binds one invocation to its pre-reserved account slot.
/// Digests are filled by `InputRecord::new` after the exact request is known.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct CallIdentity {
    pub run: String,
    pub work: String,
    pub snapshot: String,
    pub reservation: String,
    pub invocation: String,
    pub role: String,
    pub model: String,
    pub usage_authority: String,
    pub config_digest: String,
    pub driver_digest: String,
    pub semantic_digest: String,
    pub input_digest: String,
}

/// Identity fields known before the canonical request is hashed.
#[derive(Debug, Clone)]
pub(super) struct CallBinding {
    pub run: String,
    pub work: String,
    pub snapshot: String,
    pub reservation: String,
    pub invocation: String,
    pub role: String,
    pub model: String,
    pub usage_authority: String,
    pub config_digest: String,
    pub driver_digest: String,
}

/// Exact invocation-bearing request bytes, retained before dispatch. The
/// semantic digest deliberately excludes the random invocation and reservation
/// while binding Work, snapshot, config, driver, role, model, cap, and payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct InputRecord {
    pub schema: String,
    pub identity: CallIdentity,
    pub request: Value,
    pub record_digest: String,
}

impl InputRecord {
    pub(super) fn new(
        binding: CallBinding,
        request: Value,
    ) -> Result<Self, crate::error::ClewError> {
        validate_request_binding(&binding, &request)?;
        let semantic_digest = semantic_digest(&binding, &request)?;
        let input_digest = digest(&request)?;
        let identity = CallIdentity {
            run: binding.run,
            work: binding.work,
            snapshot: binding.snapshot,
            reservation: binding.reservation,
            invocation: binding.invocation,
            role: binding.role,
            model: binding.model,
            usage_authority: binding.usage_authority,
            config_digest: binding.config_digest,
            driver_digest: binding.driver_digest,
            semantic_digest,
            input_digest,
        };
        let mut record = Self {
            schema: CALL_INPUT_SCHEMA.into(),
            identity,
            request,
            record_digest: String::new(),
        };
        record.record_digest = digest(&record_without_input_digest(&record))?;
        record.validate()?;
        Ok(record)
    }

    pub(super) fn validate(&self) -> Result<(), crate::error::ClewError> {
        self.identity.validate()?;
        if self.schema != CALL_INPUT_SCHEMA {
            return Err(invalid(
                "RECOVERY_INPUT_CORRUPT: unsupported saved input schema",
            ));
        }
        let binding = self.identity.binding();
        validate_request_binding(&binding, &self.request)?;
        if digest(&self.request)? != self.identity.input_digest
            || semantic_digest(&binding, &self.request)? != self.identity.semantic_digest
            || digest(&record_without_input_digest(self))? != self.record_digest
        {
            return Err(invalid(
                "RECOVERY_INPUT_CORRUPT: saved input digest or identity is invalid",
            ));
        }
        Ok(())
    }

    pub(super) fn bounded_encoding(&self) -> Result<Vec<u8>, crate::error::ClewError> {
        bounded_bytes(self, "agent input")
    }
}

fn record_without_input_digest(record: &InputRecord) -> InputRecord {
    let mut copy = record.clone();
    copy.record_digest.clear();
    copy
}

/// Persist an input record exactly once before the account reservation changes
/// to DISPATCHED. Reusing an invocation is permitted only for byte-identical
/// content.
pub(super) fn save_input(
    repo: &Repository,
    record: &InputRecord,
) -> Result<(), crate::error::ClewError> {
    record.validate()?;
    let encoded = record.bounded_encoding()?;
    persist_immutable(
        repo,
        &input_path(&record.identity.invocation)?,
        &encoded,
        "input",
    )
}

/// Load the required input for a pending invocation. A missing file is an
/// explicit recovery error because dispatch must never precede input storage.
pub(super) fn load_input(
    repo: &Repository,
    expected: &CallIdentity,
) -> Result<InputRecord, crate::error::ClewError> {
    expected.validate()?;
    let record: InputRecord = read_required(repo, &input_path(&expected.invocation)?, "input")?;
    record.validate()?;
    if &record.identity != expected {
        return Err(invalid(
            "RECOVERY_INPUT_BINDING_MISMATCH: saved input belongs to another invocation",
        ));
    }
    Ok(record)
}

/// The canonical input stays unchanged. This separate immutable record binds
/// the exact versioned driver carrier and its role-local append-only alias map.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ModelInputRecord {
    pub schema: String,
    pub identity: CallIdentity,
    pub canonical_record_digest: String,
    pub prepared: super::super::model_ids::Prepared,
    pub carrier: Value,
    pub record_digest: String,
}

impl ModelInputRecord {
    fn validate(&self, input: &InputRecord) -> Result<(), crate::error::ClewError> {
        input.validate()?;
        let mut unsigned = self.clone();
        unsigned.record_digest.clear();
        let scope = super::super::model_ids::Scope {
            work: input.identity.work.clone(),
            run: input.identity.run.clone(),
            role: input.identity.role.clone(),
        };
        if self.schema != MODEL_INPUT_SCHEMA
            || self.identity != input.identity
            || self.canonical_record_digest != input.record_digest
            || self.prepared.scope != scope
            || self.carrier != model_carrier(&input.request, &self.prepared)
            || digest(&unsigned)? != self.record_digest
        {
            return Err(invalid(
                "RECOVERY_MODEL_INPUT_MISMATCH: carrier or scoped map binding changed",
            ));
        }
        super::super::model_ids::validate(&input.request, &self.prepared)
            .map_err(|error| invalid(format!("RECOVERY_MODEL_INPUT_CORRUPT: {}", error.message)))
    }
}

fn model_carrier(canonical: &Value, prepared: &super::super::model_ids::Prepared) -> Value {
    json!({"schema":super::super::model_ids::VERSION,"canonicalJob":canonical,"preparedModel":prepared})
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ModelMapHead {
    schema: String,
    scope: super::super::model_ids::Scope,
    config_digest: String,
    driver_digest: String,
    invocation: String,
    model_input_digest: String,
    map_digest: String,
}

fn model_input_path(invocation: &str) -> Result<String, crate::error::ClewError> {
    if !is_hex(invocation, 32) {
        return Err(invalid(
            "RECOVERY_INVOCATION_INVALID: malformed model-input invocation",
        ));
    }
    Ok(format!(".codeclew/job-model-inputs/{invocation}.json"))
}

fn model_head_path(identity: &CallIdentity) -> Result<String, crate::error::ClewError> {
    identity.validate()?;
    Ok(format!(
        ".codeclew/model-id-maps/{}/{}/head.json",
        identity.run, identity.role
    ))
}

pub(super) fn try_load_model_input(
    repo: &Repository,
    input: &InputRecord,
) -> Result<Option<ModelInputRecord>, crate::error::ClewError> {
    let Some(record) = read_optional::<ModelInputRecord>(
        repo,
        &model_input_path(&input.identity.invocation)?,
        "model input",
    )?
    else {
        return Ok(None);
    };
    record.validate(input)?;
    Ok(Some(record))
}

pub(super) fn prepare_model_input(
    repo: &Repository,
    input: &InputRecord,
    previous_invocation: Option<&str>,
) -> Result<ModelInputRecord, crate::error::ClewError> {
    if let Some(saved) = try_load_model_input(repo, input)? {
        publish_model_head(repo, input, &saved)?;
        return Ok(saved);
    }
    let scope = super::super::model_ids::Scope {
        work: input.identity.work.clone(),
        run: input.identity.run.clone(),
        role: input.identity.role.clone(),
    };
    let head_path = model_head_path(&input.identity)?;
    let previous = if let Some(head) =
        read_optional::<ModelMapHead>(repo, &head_path, "model map head")?
    {
        if head.schema != super::super::model_ids::VERSION
            || head.scope != scope
            || head.config_digest != input.identity.config_digest
            || head.driver_digest != input.identity.driver_digest
        {
            return Err(invalid(
                "RECOVERY_MODEL_MAP_MISMATCH: previous map belongs to another role or configuration",
            ));
        }
        let saved: ModelInputRecord = read_required(
            repo,
            &model_input_path(&head.invocation)?,
            "previous model input",
        )?;
        let canonical = load_input(repo, &saved.identity)?;
        saved.validate(&canonical)?;
        if saved.record_digest != head.model_input_digest
            || saved.prepared.map_digest != head.map_digest
            || saved.prepared.scope != scope
            || saved.identity.config_digest != head.config_digest
            || saved.identity.driver_digest != head.driver_digest
        {
            return Err(invalid(
                "RECOVERY_MODEL_MAP_MISMATCH: previous immutable map binding changed",
            ));
        }
        Some(saved.prepared.map)
    } else {
        if previous_invocation.is_some() {
            return Err(invalid(
                "RECOVERY_MODEL_MAP_MISSING: previous role call requires its append-only map head",
            ));
        }
        None
    };
    if let Some(invocation) = previous_invocation {
        let prior: ModelInputRecord = read_required(
            repo,
            &model_input_path(invocation)?,
            "previous retained role model input",
        )?;
        let canonical = load_input(repo, &prior.identity)?;
        prior.validate(&canonical)?;
        if prior.prepared.scope != scope
            || prior.identity.config_digest != input.identity.config_digest
            || prior.identity.driver_digest != input.identity.driver_digest
            || previous
                .as_ref()
                .is_none_or(|map| !map.entries.starts_with(&prior.prepared.map.entries))
        {
            return Err(invalid(
                "RECOVERY_MODEL_MAP_MISMATCH: map head rewinds the retained role call history",
            ));
        }
    }
    let prepared = super::super::model_ids::prepare(&input.request, &scope, previous.as_ref())?;
    let mut record = ModelInputRecord {
        schema: MODEL_INPUT_SCHEMA.into(),
        identity: input.identity.clone(),
        canonical_record_digest: input.record_digest.clone(),
        carrier: model_carrier(&input.request, &prepared),
        prepared,
        record_digest: String::new(),
    };
    record.record_digest = digest(&record)?;
    record.validate(input)?;
    persist_immutable(
        repo,
        &model_input_path(&input.identity.invocation)?,
        &bounded_bytes(&record, "model input")?,
        "model input",
    )?;
    publish_model_head(repo, input, &record)?;
    Ok(record)
}

fn publish_model_head(
    repo: &Repository,
    input: &InputRecord,
    record: &ModelInputRecord,
) -> Result<(), crate::error::ClewError> {
    #[cfg(test)]
    if INTERRUPT_MODEL_HEAD.with(|flag| flag.replace(false)) {
        return Err(invalid(
            "RECOVERY_MODEL_HEAD_PUBLICATION_INTERRUPTED: retained carrier precedes role-map head",
        ));
    }
    let scope = record.prepared.scope.clone();
    let head_path = model_head_path(&input.identity)?;
    let _guard = repo.lock()?;
    if let Some(previous) = read_optional::<ModelMapHead>(repo, &head_path, "model map head")? {
        if previous.schema != super::super::model_ids::VERSION
            || previous.scope != scope
            || previous.config_digest != input.identity.config_digest
            || previous.driver_digest != input.identity.driver_digest
        {
            return Err(invalid(
                "RECOVERY_MODEL_MAP_MISMATCH: map head changed scope or configuration",
            ));
        }
        let saved: ModelInputRecord = read_required(
            repo,
            &model_input_path(&previous.invocation)?,
            "previous model input",
        )?;
        let canonical = load_input(repo, &saved.identity)?;
        saved.validate(&canonical)?;
        if saved.record_digest != previous.model_input_digest
            || saved.prepared.map_digest != previous.map_digest
            || saved.prepared.scope != scope
            || saved.identity.config_digest != input.identity.config_digest
            || saved.identity.driver_digest != input.identity.driver_digest
        {
            return Err(invalid(
                "RECOVERY_MODEL_MAP_MISMATCH: map head no longer binds its immutable map",
            ));
        }
        let before = &saved.prepared.map.entries;
        let after = &record.prepared.map.entries;
        if before.starts_with(after) && before.len() > after.len() {
            return Ok(());
        }
        if !after.starts_with(before) {
            return Err(invalid(
                "RECOVERY_MODEL_MAP_MISMATCH: alias map is not an append-only continuation",
            ));
        }
    }
    let head = ModelMapHead {
        schema: super::super::model_ids::VERSION.into(),
        scope,
        config_digest: input.identity.config_digest.clone(),
        driver_digest: input.identity.driver_digest.clone(),
        invocation: input.identity.invocation.clone(),
        model_input_digest: record.record_digest.clone(),
        map_digest: record.prepared.map_digest.clone(),
    };
    repo.atomic(&head_path, &bounded_bytes(&head, "model map head")?)?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ModelResultBinding {
    pub version: String,
    pub model_input_digest: String,
    pub map_digest: String,
    pub raw_record_digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct RawModelResult {
    schema: String,
    identity: CallIdentity,
    model_input_digest: String,
    map_digest: String,
    pub output: Value,
    pub stdout_bytes: usize,
    pub stderr_bytes: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<String>,
    output_digest: String,
    record_digest: String,
}

impl RawModelResult {
    pub(super) fn binding(&self) -> ModelResultBinding {
        ModelResultBinding {
            version: super::super::model_ids::VERSION.into(),
            model_input_digest: self.model_input_digest.clone(),
            map_digest: self.map_digest.clone(),
            raw_record_digest: self.record_digest.clone(),
        }
    }
    fn validate(&self, model: &ModelInputRecord) -> Result<(), crate::error::ClewError> {
        let mut unsigned = self.clone();
        unsigned.record_digest.clear();
        if self.schema != RAW_MODEL_RESULT_SCHEMA
            || self.identity != model.identity
            || self.model_input_digest != model.record_digest
            || self.map_digest != model.prepared.map_digest
            || digest(&self.output)? != self.output_digest
            || digest(&unsigned)? != self.record_digest
        {
            return Err(invalid(
                "RECOVERY_MODEL_RESULT_CORRUPT: raw delivered carrier result binding changed",
            ));
        }
        Ok(())
    }
}

fn raw_model_result_path(invocation: &str) -> Result<String, crate::error::ClewError> {
    model_input_path(invocation)?;
    Ok(format!(
        ".codeclew/job-model-wire-results/{invocation}.json"
    ))
}

pub(super) fn save_raw_model_result(
    repo: &Repository,
    model: &ModelInputRecord,
    output: Value,
    stdout_bytes: usize,
    stderr_bytes: usize,
    failure: Option<String>,
) -> Result<RawModelResult, crate::error::ClewError> {
    let mut raw = RawModelResult {
        schema: RAW_MODEL_RESULT_SCHEMA.into(),
        identity: model.identity.clone(),
        model_input_digest: model.record_digest.clone(),
        map_digest: model.prepared.map_digest.clone(),
        output_digest: digest(&output)?,
        output,
        stdout_bytes,
        stderr_bytes,
        failure,
        record_digest: String::new(),
    };
    raw.record_digest = digest(&raw)?;
    raw.validate(model)?;
    persist_immutable(
        repo,
        &raw_model_result_path(&model.identity.invocation)?,
        &bounded_bytes(&raw, "raw model result")?,
        "raw model result",
    )?;
    #[cfg(test)]
    if INTERRUPT_RAW_MODEL_RESULT.with(|flag| flag.replace(false)) {
        return Err(invalid(
            "RECOVERY_MODEL_WIRE_RESULT_INTERRUPTED: raw output retained before typed decoding",
        ));
    }
    Ok(raw)
}

pub(super) fn try_load_raw_model_result(
    repo: &Repository,
    model: &ModelInputRecord,
) -> Result<Option<RawModelResult>, crate::error::ClewError> {
    let Some(raw) = read_optional::<RawModelResult>(
        repo,
        &raw_model_result_path(&model.identity.invocation)?,
        "raw model result",
    )?
    else {
        return Ok(None);
    };
    raw.validate(model)?;
    Ok(Some(raw))
}

/// A validated outer transport reply and its raw semantic result. The caller
/// must check Reply schema, invocation, role, and model before calling
/// `save_result`; semantic proposal/review validation happens after this durable
/// record exists so an invalid but delivered result is never requested twice.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct SavedResult {
    pub schema: String,
    pub identity: CallIdentity,
    pub transport_status: String,
    pub usage: Option<Usage>,
    pub result_digest: String,
    pub result: Value,
    pub stdout_bytes: usize,
    pub stderr_bytes: usize,
    pub record_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_binding: Option<ModelResultBinding>,
}

impl SavedResult {
    fn validate(&self, expected: &CallIdentity) -> Result<(), crate::error::ClewError> {
        self.identity.validate()?;
        if self.schema != CALL_RESULT_SCHEMA
            || self.transport_status != VALIDATED_REPLY
            || &self.identity != expected
            || digest(&self.result)? != self.result_digest
            || digest(&record_without_result_digest(self))? != self.record_digest
        {
            return Err(invalid(
                "RECOVERY_RESULT_CORRUPT: saved result binding, status, or digest is invalid",
            ));
        }
        Ok(())
    }
}

fn record_without_result_digest(record: &SavedResult) -> SavedResult {
    let mut copy = record.clone();
    copy.record_digest.clear();
    copy
}

/// Save validated transport output before reconciling its account reservation.
/// Existing content is accepted only when the entire canonical record matches.
#[cfg(test)]
pub(super) fn save_result(
    repo: &Repository,
    input: &InputRecord,
    usage: Option<Usage>,
    result: Value,
    stdout_bytes: usize,
    stderr_bytes: usize,
) -> Result<SavedResult, crate::error::ClewError> {
    save_result_with_model_binding(repo, input, usage, result, stdout_bytes, stderr_bytes, None)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn save_result_with_model_binding(
    repo: &Repository,
    input: &InputRecord,
    usage: Option<Usage>,
    result: Value,
    stdout_bytes: usize,
    stderr_bytes: usize,
    model_binding: Option<ModelResultBinding>,
) -> Result<SavedResult, crate::error::ClewError> {
    input.validate()?;
    let mut saved = SavedResult {
        schema: CALL_RESULT_SCHEMA.into(),
        identity: input.identity.clone(),
        transport_status: VALIDATED_REPLY.into(),
        usage,
        result_digest: digest(&result)?,
        result,
        stdout_bytes,
        stderr_bytes,
        record_digest: String::new(),
        model_binding,
    };
    saved.record_digest = digest(&record_without_result_digest(&saved))?;
    let encoded = bounded_bytes(&saved, "agent result")?;
    persist_immutable(
        repo,
        &result_path(&saved.identity.invocation)?,
        &encoded,
        "result",
    )?;
    Ok(saved)
}

/// Return `None` only when no result file exists. A present malformed, changed,
/// or insufficiently bound record is an error and must never trigger a retry.
pub(super) fn try_load_result(
    repo: &Repository,
    input: &InputRecord,
) -> Result<Option<SavedResult>, crate::error::ClewError> {
    input.validate()?;
    let path = result_path(&input.identity.invocation)?;
    let Some(saved) = read_optional::<SavedResult>(repo, &path, "result")? else {
        return Ok(None);
    };
    saved.validate(&input.identity)?;
    let model = try_load_model_input(repo, input)?;
    match (&saved.model_binding, model) {
        (None, None) => {}
        (Some(binding), Some(model)) => {
            let raw=try_load_raw_model_result(repo,&model)?.ok_or_else(||invalid("RECOVERY_MODEL_RESULT_MISSING: canonical result requires retained raw wire output"))?;
            if *binding != raw.binding() {
                return Err(invalid(
                    "RECOVERY_MODEL_RESULT_CORRUPT: canonical result is bound to another raw wire result",
                ));
            }
            if raw.failure.is_some() {
                return Err(invalid(
                    "RECOVERY_MODEL_RESULT_CORRUPT: canonical result cannot complete a failed adapter transport",
                ));
            }
            let reply: super::Reply = serde_json::from_value(raw.output.clone()).map_err(|_| {
                invalid("RECOVERY_MODEL_RESULT_CORRUPT: retained wire reply is invalid")
            })?;
            if reply.schema != "codeclew-documentation-agent-result/1.0"
                || reply.invocation != input.identity.invocation
                || reply.role != input.identity.role
                || reply.model != input.identity.model
                || reply.usage != saved.usage
                || super::super::model_ids::decode_result(&reply.result, &model.prepared).map_err(
                    |error| invalid(format!("RECOVERY_MODEL_RESULT_CORRUPT: {}", error.message)),
                )? != saved.result
            {
                return Err(invalid(
                    "RECOVERY_MODEL_RESULT_CORRUPT: canonical result differs from retained wire decoding",
                ));
            }
        }
        _ => {
            return Err(invalid(
                "RECOVERY_MODEL_RESULT_CORRUPT: model representation result binding is absent or unexpected",
            ));
        }
    }
    Ok(Some(saved))
}

/// Require a result when the selected checkpoint says it was saved. Absence is
/// distinct from the `DISPATCHED`/no-result retry case handled by the caller.
pub(super) fn load_result(
    repo: &Repository,
    input: &InputRecord,
) -> Result<SavedResult, crate::error::ClewError> {
    try_load_result(repo, input)?.ok_or_else(|| {
        invalid("RECOVERY_RESULT_MISSING: checkpoint requires a durable result that is absent")
    })
}

fn validate_request_binding(
    binding: &CallBinding,
    request: &Value,
) -> Result<(), crate::error::ClewError> {
    validate_binding_fields(binding)?;
    if request["schema"] != "codeclew-documentation-agent-job/1.0"
        || request["invocation"] != binding.invocation
        || request["role"] != binding.role
        || request["model"] != binding.model
        || request["work"] != binding.work
        || !request["cap"].is_object()
        || !request["payload"].is_object()
    {
        return Err(invalid(
            "RECOVERY_INPUT_BINDING_MISMATCH: request does not match its invocation identity",
        ));
    }
    Ok(())
}

fn semantic_digest(
    binding: &CallBinding,
    request: &Value,
) -> Result<String, crate::error::ClewError> {
    digest(&json!({
        "work": binding.work,
        "snapshot": binding.snapshot,
        "configDigest": binding.config_digest,
        "driverDigest": binding.driver_digest,
        "role": binding.role,
        "model": binding.model,
        "usageAuthority": binding.usage_authority,
        "cap": request["cap"],
        "payload": request["payload"],
    }))
}

impl CallIdentity {
    fn binding(&self) -> CallBinding {
        CallBinding {
            run: self.run.clone(),
            work: self.work.clone(),
            snapshot: self.snapshot.clone(),
            reservation: self.reservation.clone(),
            invocation: self.invocation.clone(),
            role: self.role.clone(),
            model: self.model.clone(),
            usage_authority: self.usage_authority.clone(),
            config_digest: self.config_digest.clone(),
            driver_digest: self.driver_digest.clone(),
        }
    }

    fn validate(&self) -> Result<(), crate::error::ClewError> {
        validate_binding_fields(&self.binding())?;
        if !valid_digest(&self.semantic_digest) || !valid_digest(&self.input_digest) {
            return Err(invalid(
                "RECOVERY_CALL_BINDING_INVALID: invalid request digest",
            ));
        }
        Ok(())
    }
}

fn validate_binding_fields(binding: &CallBinding) -> Result<(), crate::error::ClewError> {
    validate_run_id(&binding.run)?;
    let _snapshot = parse_snapshot_ref(&binding.snapshot)?;
    if !is_hex(&binding.work, 64)
        || !is_hex(&binding.invocation, 32)
        || !valid_digest(&binding.reservation)
        || !valid_digest(&binding.config_digest)
        || !valid_digest(&binding.driver_digest)
        || !matches!(binding.role.as_str(), "author" | "reviewer" | "fallback")
        || binding.model.trim().is_empty()
        || binding.model.len() > 256
        || !matches!(
            binding.usage_authority.as_str(),
            "MAXIMUM_ONLY" | "TRANSPORT_METADATA"
        )
    {
        return Err(invalid(
            "RECOVERY_CALL_BINDING_INVALID: malformed invocation identity",
        ));
    }
    Ok(())
}

/// Parse the `digest/size` handle emitted by `Check::retained` and consumed by
/// `Check::load_snapshot_manifest`. The report carries no ObjectRef schema, so
/// the current check-manifest schema is restored here; Work admission remains
/// responsible for proving that this reference names a retained store object.
fn parse_snapshot_ref(
    snapshot: &str,
) -> Result<super::super::cache::ObjectRef, crate::error::ClewError> {
    let (digest, size) = snapshot.rsplit_once('/').ok_or_else(|| {
        invalid("RECOVERY_CALL_BINDING_INVALID: snapshot must be a sha256 identity/size handle")
    })?;
    let size = size
        .parse::<u64>()
        .map_err(|_| invalid("RECOVERY_CALL_BINDING_INVALID: snapshot size is invalid"))?;
    if !valid_digest(digest)
        || size > super::super::check::PORTABLE_CACHE_MAX_BYTES
        || snapshot != format!("{digest}/{size}")
    {
        return Err(invalid(
            "RECOVERY_CALL_BINDING_INVALID: snapshot reference is malformed or oversized",
        ));
    }
    Ok(super::super::cache::ObjectRef::new(
        super::super::check::CHECK_MANIFEST_SCHEMA.into(),
        digest.into(),
        size,
    ))
}

fn validate_run_id(run: &str) -> Result<(), crate::error::ClewError> {
    if !is_hex(run, 32) {
        return Err(invalid("RECOVERY_RUN_ID_INVALID: malformed run identity"));
    }
    Ok(())
}

fn validate_checkpoint_ref(reference: &CheckpointRef) -> Result<(), crate::error::ClewError> {
    validate_run_id(&reference.run)?;
    if reference.schema != CHECKPOINT_REF_SCHEMA || !valid_digest(&reference.checkpoint_digest) {
        return Err(invalid(
            "RECOVERY_CHECKPOINT_REF_INVALID: malformed checkpoint reference",
        ));
    }
    Ok(())
}

fn is_hex(value: &str, length: usize) -> bool {
    value.len() == length && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn valid_digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

fn checkpoint_path(
    run: &str,
    sequence: u64,
    checkpoint_digest: &str,
) -> Result<String, crate::error::ClewError> {
    validate_run_id(run)?;
    if !valid_digest(checkpoint_digest) {
        return Err(invalid(
            "RECOVERY_CHECKPOINT_REF_INVALID: malformed checkpoint digest",
        ));
    }
    Ok(format!(
        ".codeclew/jobs/{run}/checkpoints/{sequence:016x}-{}.json",
        &checkpoint_digest[7..]
    ))
}

fn input_path(invocation: &str) -> Result<String, crate::error::ClewError> {
    if !is_hex(invocation, 32) {
        return Err(invalid(
            "RECOVERY_INVOCATION_INVALID: malformed invocation identity",
        ));
    }
    Ok(format!(".codeclew/job-inputs/{invocation}.json"))
}

fn result_path(invocation: &str) -> Result<String, crate::error::ClewError> {
    if !is_hex(invocation, 32) {
        return Err(invalid(
            "RECOVERY_INVOCATION_INVALID: malformed invocation identity",
        ));
    }
    // Keep the established audit path while strengthening the stored binding.
    Ok(format!(".codeclew/job-results/{invocation}.json"))
}

fn bounded_bytes<T: Serialize>(value: &T, label: &str) -> Result<Vec<u8>, crate::error::ClewError> {
    let encoded = bytes(value)?;
    if encoded.len() > MAX_RECOVERY_BYTES {
        return Err(invalid(format!(
            "{label} exceeds the recovery record bound"
        )));
    }
    Ok(encoded)
}

fn persist_immutable(
    repo: &Repository,
    relative: &str,
    encoded: &[u8],
    label: &str,
) -> Result<(), crate::error::ClewError> {
    let guard = repo.lock()?;
    persist_immutable_locked(repo, &guard, relative, encoded, label)
}

fn persist_immutable_locked(
    repo: &Repository,
    _guard: &WriteLock,
    relative: &str,
    encoded: &[u8],
    label: &str,
) -> Result<(), crate::error::ClewError> {
    if encoded.len() > MAX_RECOVERY_BYTES {
        return Err(invalid(format!(
            "{label} exceeds the recovery record bound"
        )));
    }
    let path = repo.path(relative)?;
    match fs::symlink_metadata(&path) {
        Ok(metadata) => {
            if !metadata.is_file() || metadata.len() > MAX_RECOVERY_BYTES as u64 {
                return Err(invalid(format!(
                    "RECOVERY_RECORD_CORRUPT: {label} is not a bounded regular file"
                )));
            }
            let existing = fs::read(&path).map_err(io_error)?;
            if existing != encoded {
                return Err(invalid(format!(
                    "RECOVERY_IMMUTABLE_CONFLICT: {label} already exists with different content"
                )));
            }
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            repo.atomic(relative, encoded)
        }
        Err(error) => Err(io_error(error)),
    }
}

fn read_required<T: DeserializeOwned>(
    repo: &Repository,
    relative: &str,
    label: &str,
) -> Result<T, crate::error::ClewError> {
    read_optional(repo, relative, label)?.ok_or_else(|| {
        invalid(format!(
            "RECOVERY_RECORD_MISSING: required {label} record is absent"
        ))
    })
}

fn read_optional<T: DeserializeOwned>(
    repo: &Repository,
    relative: &str,
    label: &str,
) -> Result<Option<T>, crate::error::ClewError> {
    let path = repo.path(relative)?;
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(io_error(error)),
    };
    if !metadata.is_file() || metadata.len() > MAX_RECOVERY_BYTES as u64 {
        return Err(invalid(format!(
            "RECOVERY_RECORD_CORRUPT: {label} is not a bounded regular file"
        )));
    }
    store::read(&path, MAX_RECOVERY_BYTES as u64)
        .map(Some)
        .map_err(|_| {
            invalid(format!(
                "RECOVERY_RECORD_CORRUPT: {label} cannot be decoded"
            ))
        })
}
