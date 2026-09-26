//! Immutable, bounded records used to resume one documentation-agent run.
//!
//! This module stores recovery material only. The parent coordinator owns the
//! state machine and selects a checkpoint by atomically saving its returned
//! `CheckpointRef` in the run report. Unselected checkpoint files are harmless
//! orphans after a crash.

use super::super::{bytes, digest, invalid, io_error};
use super::{
    Usage,
    store::{self, Repository},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::fs;

const CHECKPOINT_REF_SCHEMA: &str = "codeclew-documentation-recovery-checkpoint-ref/1.0";
const CHECKPOINT_FILE_SCHEMA: &str = "codeclew-documentation-recovery-checkpoint/1.0";
const CALL_INPUT_SCHEMA: &str = "codeclew-documentation-agent-input/1.0";
const CALL_RESULT_SCHEMA: &str = "codeclew-documentation-agent-result-record/2.0";
const VALIDATED_REPLY: &str = "VALIDATED_REPLY";
const MAX_RECOVERY_BYTES: usize = store::MAX_RECORD as usize;

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

/// Persist a new immutable checkpoint. Save the returned reference in the run
/// report in a separate atomic write; a crash between those writes leaves only
/// an unselected immutable file.
pub(super) fn save_checkpoint<T: Serialize>(
    repo: &Repository,
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
    persist_immutable(repo, &path, &encoded, "checkpoint")?;
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
    let encoded = bounded_bytes(record, "agent input")?;
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
pub(super) fn save_result(
    repo: &Repository,
    input: &InputRecord,
    usage: Option<Usage>,
    result: Value,
    stdout_bytes: usize,
    stderr_bytes: usize,
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
    if encoded.len() > MAX_RECOVERY_BYTES {
        return Err(invalid(format!(
            "{label} exceeds the recovery record bound"
        )));
    }
    let _lock = repo.lock()?;
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
