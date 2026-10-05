//! Independent grouped evidence retrieval for explicitly expanding operation reviews.
//! The author packet and answer remain immutable across every reviewer call.

use super::{
    MeaningReview, Origin, RetryOrigin, ReviewConfig, account, digest, invalid, payload, preflight,
    schema, summary, validate_config, validate_review, verdict_status,
};
use crate::documentation::{
    agent_jobs::{self, Config, Role, RunCheckpoint, RunReport, recorded_expansion, recovery},
    operation_context,
    store::{self, Repository},
    work::{self, Selection, Work},
};
use crate::error::ClewError;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

pub(super) const MODE: &str = "OPERATION_DRAFT_REVIEW/1.1";
pub(super) const CONFIG_SCHEMA: &str =
    "codeclew-documentation-operation-draft-review-execution/1.1";
const REVIEW_SCHEMA: &str = "codeclew-operation-draft-meaning-review/1.1";

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ContextConfig {
    schema: String,
    reviewer: Role,
    reviewer_calls: u32,
    budget: agent_jobs::Budget,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ExpansionStep {
    identity: recovery::CallIdentity,
    before_pages: usize,
    before_parts: usize,
    after_pages: usize,
    after_parts: usize,
    result_digest: String,
    lookup_feedback: Vec<Value>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PendingExpansion {
    identity: recovery::CallIdentity,
    selections: Vec<Selection>,
    before_pages: usize,
    before_parts: usize,
    result_digest: String,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DeliveryState {
    configured_calls: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    seed: Option<RetrySeed>,
    steps: Vec<ExpansionStep>,
    pending: Option<PendingExpansion>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RetrySeed {
    pages: usize,
    parts: usize,
    lookup_feedback: Vec<Value>,
    expansion_feedback: Option<Value>,
}

fn state(checkpoint: &RunCheckpoint) -> Result<DeliveryState, ClewError> {
    serde_json::from_value(checkpoint.previous_section.clone())
        .map_err(|_| invalid("RECOVERY_CONTEXT_MISMATCH: reviewer delivery state is invalid"))
}

fn set_state(checkpoint: &mut RunCheckpoint, state: &DeliveryState) -> Result<(), ClewError> {
    checkpoint.previous_section =
        serde_json::to_value(state).map_err(crate::documentation::io_error)?;
    Ok(())
}

/// Selection bounds are applied to native reads by the shared coordinator. The
/// role action itself may group any number of exact references or symbols.
fn selection_schema() -> Value {
    json!({"type":"object","additionalProperties":false,"properties":{
        "references":{"type":"array","items":{"type":"string","minLength":1}},
        "symbols":{"type":"array","items":{"type":"string","minLength":1}},
        "query":{"anyOf":[{"type":"null"},{"type":"object","additionalProperties":false,"required":["kind"],"properties":{
            "kind":{"type":"string"},"symbolContains":{"type":"string"},
            "projection":{"enum":["RAW","NAVIGATION"]}}}]},
        "cursor":{"type":["string","null"]},"untrackedReads":{"const":false}}})
}

#[allow(clippy::too_many_arguments)]
fn context_payload(
    repo: &Repository,
    work: &Work,
    origin: &Origin,
    packet: &Value,
    answer: &Value,
    blocks: &[Value],
    author_contract: &Value,
    pages: &[Value],
    parts: &[Value],
    lookup_feedback: &[Value],
    configured_calls: u32,
    completed_calls: usize,
) -> Result<Value, ClewError> {
    let remaining_calls = configured_calls
        .checked_sub(u32::try_from(completed_calls).map_err(|_| {
            invalid("RECOVERY_CONTEXT_MISMATCH: reviewer call ordinal exceeds configured budget")
        })?)
        .filter(|remaining| *remaining > 0)
        .ok_or_else(|| {
            invalid("RECOVERY_CONTEXT_MISMATCH: reviewer request has no configured calls remaining")
        })?;
    let context = operation_context::context(repo, work, pages, parts)?;
    let mut value = payload(work, origin, packet, answer, blocks, author_contract)?;
    let instruction = value["instruction"].as_str().unwrap_or_default().replace(
        "Do not rewrite the answer, invoke tools, ask for expansion or publish.",
        "Do not rewrite the answer or publish. You may request registered grouped evidence expansion independently of the author's choices.",
    ).replace(
        "Return exactly outputSchema with the supplied binding and complete assessedBlocks/assessedEvidence sets.",
        "Return a review action with the supplied binding and complete assessedBlocks/assessedEvidence sets, or an expand action containing grouped native selections. The host retrieves complete content pages and source parts; do not make a separate request for each technical part. SYMBOL queries return declaration navigation: navigation-only cards are not citable evidence. Request complete declarations and their related source when a material helper, argument origin, guard, mutation or failure needs clarification. Cite only packet.citations and fully delivered reviewContext.citations. Keep unavailable evidence explicit. The final review must bind reviewContextDigest to the exact deliveredDigest supplied in this call.",
    );
    value["instruction"] = json!(format!(
        "{instruction} Your configured budget is {configured_calls} reviewer calls; {remaining_calls} remain including this decision. Expansion decisions and the final verdict each consume one call. Return a verdict once sufficient evidence is delivered, and keep unresolved gaps explicit on your last call. Use selectionGuidance for exact native handles and declaration discovery."
    ));
    let mut labels: Vec<String> = packet["citations"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(key, _)| key.clone())
        .collect();
    labels.extend(
        context["citations"]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(key, _)| key.clone()),
    );
    labels.sort();
    labels.dedup();
    let mut review_schema = schema(work, origin, blocks, &labels)?;
    let mut definitions = review_schema
        .as_object_mut()
        .unwrap()
        .remove("$defs")
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    review_schema["properties"]["schema"] = json!({"const":REVIEW_SCHEMA});
    review_schema["properties"]["reviewContextDigest"] =
        json!({"const":context["deliveredDigest"]});
    review_schema["required"]
        .as_array_mut()
        .unwrap()
        .push(json!("reviewContextDigest"));
    definitions.insert("review".into(), review_schema);
    let output = json!({"type":"object","oneOf":[
        {"type":"object","additionalProperties":false,"required":["action","review"],
            "properties":{"action":{"const":"review"},"review":{"$ref":"#/$defs/review"}}},
        {"type":"object","additionalProperties":false,"required":["action","selections"],
            "properties":{"action":{"const":"expand"},"selections":{"type":"array","minItems":1,"items":selection_schema()}}}
    ],"$defs":definitions});
    value["lookupFeedback"] = json!(lookup_feedback);
    value["selectionGuidance"] = agent_jobs::selection_guidance(work);
    value["roleBudget"] =
        json!({"configuredCalls":configured_calls,"remainingCalls":remaining_calls});
    value["reviewContext"] = context;
    value["evidenceKeys"] = json!(labels);
    value["outputSchema"] = output;
    Ok(value)
}

pub(super) fn validate_context_review(
    value: Value,
    work: &Work,
    origin: &Origin,
    packet: &Value,
    blocks: &[Value],
    context: &Value,
) -> Result<MeaningReview, ClewError> {
    if value["schema"] != REVIEW_SCHEMA
        || value["reviewContextDigest"] != context["deliveredDigest"]
    {
        return Err(invalid(
            "DRAFT_REVIEW_BINDING_MISMATCH: review does not bind independent delivered context",
        ));
    }
    let mut normalized = value;
    normalized
        .as_object_mut()
        .ok_or_else(|| invalid("invalid expanding review"))?
        .remove("reviewContextDigest");
    normalized["schema"] = json!(super::REVIEW_SCHEMA);
    let mut evidence_packet = packet.clone();
    for (key, value) in context["citations"].as_object().into_iter().flatten() {
        evidence_packet["citations"]
            .as_object_mut()
            .unwrap()
            .entry(key.clone())
            .or_insert_with(|| value.clone());
    }
    validate_review(normalized, work, origin, &evidence_packet, blocks)
}

fn validate_action(value: &Value, action: &str, field: &str) -> Result<(), ClewError> {
    let object = value
        .as_object()
        .ok_or_else(|| invalid("DRAFT_REVIEW_INVALID_RESULT: action must be an object"))?;
    if value["action"] != action || object.len() != 2 || !object.contains_key(field) {
        return Err(invalid(
            "DRAFT_REVIEW_INVALID_RESULT: use exactly one review or grouped expand action",
        ));
    }
    Ok(())
}

pub(super) fn unwrap_review_action(value: &Value) -> Result<Value, ClewError> {
    validate_action(value, "review", "review")?;
    Ok(value["review"].clone())
}

fn selections(value: &Value) -> Result<Vec<Selection>, ClewError> {
    validate_action(value, "expand", "selections")?;
    let selected: Vec<Selection> = serde_json::from_value(value["selections"].clone())
        .map_err(|_| invalid("DRAFT_REVIEW_INVALID_RESULT: invalid grouped selections"))?;
    if selected.is_empty() {
        return Err(invalid(
            "DRAFT_REVIEW_INVALID_RESULT: empty grouped expansion",
        ));
    }
    for selection in &selected {
        if selection.untracked_reads
            || selection
                .references
                .iter()
                .chain(&selection.symbols)
                .any(|value| value.trim().is_empty())
        {
            return Err(invalid(
                "DRAFT_REVIEW_INVALID_RESULT: use exact tracked nonempty selectors",
            ));
        }
        native_selections(selection)?;
    }
    Ok(selected)
}

fn native_selections(selection: &Selection) -> Result<Vec<Selection>, ClewError> {
    if (!selection.references.is_empty() && !selection.symbols.is_empty())
        || (selection.query.is_some()
            && (!selection.references.is_empty() || !selection.symbols.is_empty()))
    {
        return Err(invalid(
            "DRAFT_REVIEW_INVALID_RESULT: select references, symbols or one query separately",
        ));
    }
    let count = selection.references.len().max(selection.symbols.len());
    // Chunk only at the existing native Work protocol boundary.
    if count <= 8 {
        work::validate_selection(selection)?;
        return Ok(vec![selection.clone()]);
    }
    if selection.cursor.is_some() {
        return Err(invalid(
            "DRAFT_REVIEW_INVALID_RESULT: a grouped cursor must belong to one native selection",
        ));
    }
    let values = if selection.references.is_empty() {
        &selection.symbols
    } else {
        &selection.references
    };
    values
        .chunks(8)
        .map(|chunk| {
            let mut native = selection.clone();
            if selection.references.is_empty() {
                native.symbols = chunk.to_vec();
            } else {
                native.references = chunk.to_vec();
            }
            work::validate_selection(&native)?;
            Ok(native)
        })
        .collect()
}

fn feedback(work: &Work, selections: &[Selection]) -> Result<Vec<Value>, ClewError> {
    let mut feedback = Vec::new();
    for selection in selections {
        for requested in native_selections(selection)? {
            let effective = recorded_expansion::effective_expansion_selection(&requested);
            if let Err(error) = work::audit_rows(work, &effective) {
                let action = json!({"action":"expand","selection":requested});
                if let Some(value) =
                    recorded_expansion::symbol_lookup_feedback(&action, &effective, &error)
                {
                    feedback.push(value);
                }
            }
        }
    }
    Ok(feedback)
}

fn last_feedback(delivery: &DeliveryState) -> &[Value] {
    delivery
        .steps
        .last()
        .map(|step| step.lookup_feedback.as_slice())
        .or_else(|| {
            delivery
                .seed
                .as_ref()
                .map(|seed| seed.lookup_feedback.as_slice())
        })
        .unwrap_or(&[])
}

fn feasible_budget(
    cfg: &ContextConfig,
    ledger: &agent_jobs::Account,
    run: &str,
) -> Result<(), ClewError> {
    let maximum = &cfg.reviewer.cap.maximum;
    let count = u64::from(cfg.reviewer_calls);
    let overflow =
        || invalid("DRAFT_REVIEW_CONFIG_INVALID: reviewerCalls reservation arithmetic overflow");
    let required = agent_jobs::Amount {
        input_tokens: maximum
            .input_tokens
            .checked_mul(count)
            .ok_or_else(overflow)?,
        output_tokens: maximum
            .output_tokens
            .checked_mul(count)
            .ok_or_else(overflow)?,
        cost_units: maximum.cost_units.checked_mul(count).ok_or_else(overflow)?,
    };
    let mut total = required;
    for reservation in ledger
        .reservations
        .values()
        .filter(|reservation| reservation.run != run)
    {
        total = total.add(&reservation.charged)?;
    }
    if !total.within(&cfg.budget.stop_loss) {
        return Err(invalid(
            "BUDGET_EXHAUSTED: configured reviewerCalls do not fit the explicit stop-loss",
        ));
    }
    Ok(())
}

struct Material<'a> {
    repo: &'a Repository,
    work: &'a Work,
    origin: &'a Origin,
    packet: &'a Value,
    answer: &'a Value,
    blocks: &'a [Value],
    author: &'a Value,
}

impl Material<'_> {
    fn payload(
        &self,
        pages: &[Value],
        parts: &[Value],
        feedback: &[Value],
        configured_calls: u32,
        completed_calls: usize,
    ) -> Result<Value, ClewError> {
        context_payload(
            self.repo,
            self.work,
            self.origin,
            self.packet,
            self.answer,
            self.blocks,
            self.author,
            pages,
            parts,
            feedback,
            configured_calls,
            completed_calls,
        )
    }
}

pub(super) fn run(
    repo: &Repository,
    work: &Work,
    source_run: &str,
    path: &Path,
    retry: Option<&str>,
) -> Result<Value, ClewError> {
    let cfg: ContextConfig = store::read(path, store::MAX_RECORD)?;
    if cfg.schema != CONFIG_SCHEMA || cfg.reviewer_calls == 0 {
        return Err(invalid(
            "DRAFT_REVIEW_CONFIG_INVALID: configure a positive finite reviewerCalls budget",
        ));
    }
    validate_config(&ReviewConfig {
        schema: super::CONFIG_SCHEMA.into(),
        reviewer: cfg.reviewer.clone(),
        budget: cfg.budget.clone(),
    })?;
    super::super::operation_draft::validate_work(work)?;
    // Admission must fail before reservations for an incompatible frozen Work.
    operation_context::context(repo, work, &[], &[])?;
    let source = agent_jobs::load_report_by_id(repo, &work.id, source_run)?;
    let (packet, audit) = super::super::operation_draft::review_packet(repo, work, &source)?;
    let (origin, answer, author) =
        super::super::operation_draft::review_source(repo, work, &source, &packet, &audit)?;
    let blocks = crate::documentation::operation_answer::review_blocks(&answer)?;
    let material = Material {
        repo,
        work,
        origin: &origin,
        packet: &packet,
        answer: &answer,
        blocks: &blocks,
        author: &author,
    };
    let selected = agent_jobs::latest_report(repo, &work.id)?;
    let fresh_retry =
        retry.is_some_and(|run| selected.as_ref().is_some_and(|report| report.run == run));
    if retry.is_some()
        && !fresh_retry
        && selected
            .as_ref()
            .is_some_and(|report| report.status != "PREPARED")
    {
        return Err(invalid(
            "DRAFT_REVIEW_RETRY_ALREADY_FINISHED: replay the existing child without --retry-from-review; no second retry is permitted",
        ));
    }
    let retry_source = if fresh_retry {
        Some(uncertain_source(&material, selected.as_ref().unwrap())?)
    } else {
        None
    };
    let ledger = account(repo, &cfg.budget)?;
    if ledger.reservations.values().any(|reservation| {
        reservation.run == source_run
            || source.draft_repair.as_ref().is_some_and(|repair| {
                repair.rejection.as_ref().is_some_and(|rejection| {
                    reservation.run == repair.source_run || reservation.run == rejection.review_run
                })
            })
    }) {
        return Err(invalid(
            "DRAFT_REVIEW_ACCOUNT_CONFLICT: use a separate review account; source and ancestor accounting must remain unchanged",
        ));
    }
    if fresh_retry && !ledger.reservations.is_empty() {
        return Err(invalid(
            "DRAFT_REVIEW_RETRY_ACCOUNT_CONFLICT: select a new empty review account; prior maximum charges are retained",
        ));
    }
    let admission = crate::documentation::agent_adapter::admit(repo, &cfg.reviewer)?;
    let drivers = BTreeMap::from([(
        "reviewer".into(),
        admission["driverDigest"]
            .as_str()
            .ok_or_else(|| invalid("reviewer admission has no driver digest"))?
            .to_owned(),
    )]);
    let config_digest = digest(&cfg)?;
    let config = Config {
        schema: "codeclew-documentation-execution/1.0".into(),
        author: cfg.reviewer.clone(),
        reviewer: cfg.reviewer.clone(),
        author_output_contract: None,
        fallback: None,
        author_calls: 0,
        reviewer_calls: cfg.reviewer_calls,
        fallback_calls: 0,
        repair_attempts: 0,
        expansions: cfg.reviewer_calls.saturating_sub(1),
        budget: cfg.budget.clone(),
    };
    let (mut report, mut checkpoint) = match selected {
        Some(report) if (report.run == source_run && retry.is_none()) || fresh_retry => {
            let report = RunReport {
                schema: "codeclew-documentation-work-run/1.0".into(),
                run: uuid::Uuid::new_v4().simple().to_string(),
                work: work.id.clone(),
                status: "PREPARED".into(),
                config_digest: Some(config_digest.clone()),
                attempts: Vec::new(),
                proposal: None,
                review: None,
                publication: Some(json!({"status":"NOT_PUBLISHED"})),
                gap: None,
                accounting: None,
                context_budget: None,
                execution_mode: Some(MODE.into()),
                draft: None,
                draft_repair: None,
                draft_review: Some(origin.clone()),
                draft_review_retry: retry_source.as_ref().map(|(origin, _)| origin.clone()),
                checkpoint: None,
            };
            let mut checkpoint = RunCheckpoint::new(
                &report,
                origin.snapshot.clone(),
                config_digest,
                drivers.clone(),
                digest(&work::read_state(repo, &work.id)?)?,
            );
            checkpoint.phase = "REVIEWER".into();
            checkpoint.previous = answer.clone();
            checkpoint.feedback = json!(blocks);
            let mut delivery = DeliveryState {
                configured_calls: cfg.reviewer_calls,
                ..DeliveryState::default()
            };
            if let Some((_, failed)) = &retry_source {
                let prior = state(failed)?;
                checkpoint.pages = failed.pages.clone();
                checkpoint.source_parts = failed.source_parts.clone();
                checkpoint.expansion_feedback = failed.expansion_feedback.clone();
                delivery.seed = Some(RetrySeed {
                    pages: failed.pages.len(),
                    parts: failed.source_parts.len(),
                    lookup_feedback: last_feedback(&prior).to_vec(),
                    expansion_feedback: failed.expansion_feedback.clone(),
                });
            }
            set_state(&mut checkpoint, &delivery)?;
            (report, checkpoint)
        }
        Some(report)
            if report.execution_mode.as_deref() == Some(MODE)
                && report.draft_review.as_ref() == Some(&origin) =>
        {
            if retry.is_some()
                && report
                    .draft_review_retry
                    .as_ref()
                    .is_none_or(|lineage| Some(lineage.review_run.as_str()) != retry)
            {
                return Err(invalid(
                    "DRAFT_REVIEW_RETRY_INELIGIBLE: selected retry does not own the requested uncertain source",
                ));
            }
            let checkpoint =
                agent_jobs::load_run_checkpoint(repo, &report, &config_digest, &drivers)?
                    .ok_or_else(|| {
                        invalid("RECOVERY_CHECKPOINT_MISSING: expanding review has no state")
                    })?;
            (report, checkpoint)
        }
        _ => {
            return Err(invalid(
                "DRAFT_REVIEW_SOURCE_STALE: select the latest successful draft or its exact expanding review child",
            ));
        }
    };
    if checkpoint.previous != answer
        || checkpoint.feedback != json!(blocks)
        || checkpoint.snapshot != origin.snapshot
    {
        return Err(invalid(
            "RECOVERY_INPUT_BINDING_MISMATCH: immutable review answer or source changed",
        ));
    }
    let mut delivery = state(&checkpoint)?;
    if delivery.configured_calls != cfg.reviewer_calls {
        return Err(invalid(
            "RECOVERY_CONFIG_MISMATCH: reviewer call budget differs from the configured path",
        ));
    }
    validate_chain(&material, &report, &checkpoint, &delivery)?;
    if checkpoint.phase == "TERMINAL" {
        validate_terminal(&material, &report, &checkpoint)?;
        agent_jobs::complete_terminal_accounting(repo, &config, &mut report)?;
        agent_jobs::validate_terminal_reservations(repo, &config, &report)?;
        return Ok(summary(&report));
    }
    let reserved = ledger
        .reservations
        .values()
        .any(|reservation| reservation.run == report.run);
    if !reserved && !report.attempts.is_empty() {
        return Err(invalid(
            "RECOVERY_ACCOUNTING_MISMATCH: reviewer attempts have no original reservations",
        ));
    }
    if !reserved && report.attempts.is_empty() {
        feasible_budget(&cfg, &ledger, &report.run)?;
        let request = material.payload(
            &checkpoint.pages,
            &checkpoint.source_parts,
            last_feedback(&delivery),
            delivery.configured_calls,
            0,
        )?;
        preflight(&report, &checkpoint, &cfg.reviewer, &request)?;
    }
    if report.draft_review_retry.is_some() {
        // Persist lineage before the atomic fresh-account guard so a crash can
        // resume only this child and cannot allocate an implicit second retry.
        agent_jobs::save_run_checkpoint(repo, &mut report, &checkpoint)?;
        agent_jobs::reserve_expanding_review_retry(repo, &config, &report.run)?;
    } else {
        agent_jobs::ensure_reserved(repo, &config, &report.run)?;
    }
    agent_jobs::save_run_checkpoint(repo, &mut report, &checkpoint)?;
    let outcome = run_loop(
        &material,
        &cfg,
        &config,
        &mut report,
        &mut checkpoint,
        &mut delivery,
    );
    if let Err(error) = outcome {
        if error.message.starts_with("RECOVERY_") {
            return Err(error);
        }
        report.status = if error.message.starts_with("DRAFT_REVIEW_INVALID_RESULT") {
            "DRAFT_REVIEW_INVALID_RESULT"
        } else if error.message.contains("CANCEL") {
            "DRAFT_REVIEW_CANCELLED"
        } else if report.attempts.last().is_some_and(|attempt| {
            attempt.result_digest.is_none()
                && matches!(
                    attempt.status.as_str(),
                    "FAILED" | "DISPATCHED" | "DISPATCH_UNCERTAIN_MAXIMUM_RETAINED"
                )
        }) || checkpoint
            .pending_call
            .as_ref()
            .is_some_and(|pending| pending.status == "UNCERTAIN_NO_RESULT")
        {
            "DRAFT_REVIEW_UNCERTAIN"
        } else {
            "DRAFT_REVIEW_FAILED"
        }
        .into();
        report.review = None;
        checkpoint.review = None;
        report.gap = Some(
            json!({"reason":error.message,"nextAction":"Inspect the exact saved role result, retained evidence and accounting. This terminal run never redispatches. An original uncertain review without a saved result may be replaced only by an explicitly selected --retry-from-review attempt with a new empty budget account."}),
        );
        set_state(&mut checkpoint, &delivery)?;
    }
    super::super::operation_draft::finish_state(repo, &config, &mut report, &mut checkpoint)?;
    Ok(summary(&report))
}

fn run_loop(
    material: &Material<'_>,
    cfg: &ContextConfig,
    config: &Config,
    report: &mut RunReport,
    checkpoint: &mut RunCheckpoint,
    delivery: &mut DeliveryState,
) -> Result<(), ClewError> {
    loop {
        if let Some(pending) = delivery.pending.clone() {
            let mut pages = checkpoint.pages.clone();
            let mut parts = checkpoint.source_parts.clone();
            let retrieval = recorded_expansion::retrieve(
                material.repo,
                material.work,
                &pending.selections,
                &mut pages,
                &mut parts,
                |_, saved_pages, saved_parts| {
                    // Save the prefix even when its role cap is exceeded: a native
                    // receipt may already have reached disk, and remains auditable.
                    checkpoint.pages = saved_pages.to_vec();
                    checkpoint.source_parts = saved_parts.to_vec();
                    checkpoint.read_digest =
                        digest(&work::read_state(material.repo, &material.work.id)?)?;
                    agent_jobs::save_run_checkpoint(material.repo, report, checkpoint)?;
                    agent_jobs::ensure_input_bytes_cap(
                        &cfg.reviewer,
                        crate::documentation::bytes(&(saved_pages, saved_parts))?.len(),
                    )
                },
            );
            // Failure may precede a callback; keep the exact newly read state for
            // terminal auditing without starting a replacement invocation.
            checkpoint.pages = pages;
            checkpoint.source_parts = parts;
            checkpoint.read_digest = digest(&work::read_state(material.repo, &material.work.id)?)?;
            let retrieval = retrieval?;
            operation_context::context(
                material.repo,
                material.work,
                &checkpoint.pages,
                &checkpoint.source_parts,
            )?;
            let expected_feedback = feedback(material.work, &pending.selections)?;
            if retrieval.lookup_feedback != expected_feedback {
                return Err(invalid(
                    "RECOVERY_CONTEXT_MISMATCH: lookup feedback differs from frozen Work",
                ));
            }
            delivery.steps.push(ExpansionStep {
                identity: pending.identity,
                before_pages: pending.before_pages,
                before_parts: pending.before_parts,
                after_pages: checkpoint.pages.len(),
                after_parts: checkpoint.source_parts.len(),
                result_digest: pending.result_digest,
                lookup_feedback: retrieval.lookup_feedback,
            });
            delivery.pending = None;
            checkpoint.pending_call = None;
            checkpoint.expansion_feedback = Some(json!({"lookupFeedback":last_feedback(delivery)}));
            checkpoint.phase = "REVIEWER".into();
            set_state(checkpoint, delivery)?;
            agent_jobs::save_run_checkpoint(material.repo, report, checkpoint)?;
        }
        if report.attempts.len() >= cfg.reviewer_calls as usize && checkpoint.pending_call.is_none()
        {
            report.status = "DRAFT_REVIEW_NEEDS_EVIDENCE".into();
            report.gap = Some(
                json!({"reason":"The configured reviewerCalls budget ended after an evidence request; no meaning verdict was issued.","nextAction":"Inspect retained evidence and accounting. Completed evidence decisions are preserved; this run cannot be implicitly replaced or exported as approved."}),
            );
            return Ok(());
        }
        let request = material.payload(
            &checkpoint.pages,
            &checkpoint.source_parts,
            last_feedback(delivery),
            delivery.configured_calls,
            delivery.steps.len(),
        )?;
        preflight(report, checkpoint, &cfg.reviewer, &request)?;
        let (value, _, _) = agent_jobs::call(
            material.repo,
            config,
            report,
            checkpoint,
            "reviewer",
            &cfg.reviewer,
            request,
            None,
            false,
            false,
        )?;
        if value["action"] == "expand" {
            let selected = selections(&value)?;
            let identity = checkpoint
                .pending_call
                .as_ref()
                .ok_or_else(|| invalid("missing reviewer invocation"))?
                .identity
                .clone();
            let result_digest = report
                .attempts
                .last()
                .and_then(|attempt| attempt.result_digest.clone())
                .ok_or_else(|| invalid("missing saved reviewer result"))?;
            delivery.pending = Some(PendingExpansion {
                identity,
                selections: selected,
                before_pages: checkpoint.pages.len(),
                before_parts: checkpoint.source_parts.len(),
                result_digest,
            });
            checkpoint.phase = "REVIEWER".into();
            set_state(checkpoint, delivery)?;
            agent_jobs::save_run_checkpoint(material.repo, report, checkpoint)?;
            continue;
        }
        validate_action(&value, "review", "review")?;
        let context = operation_context::context(
            material.repo,
            material.work,
            &checkpoint.pages,
            &checkpoint.source_parts,
        )?;
        let review = validate_context_review(
            value["review"].clone(),
            material.work,
            material.origin,
            material.packet,
            material.blocks,
            &context,
        )
        .map_err(|error| invalid(format!("DRAFT_REVIEW_INVALID_RESULT: {}", error.message)))?;
        report.status = verdict_status(&review.verdict).into();
        report.review = Some(value["review"].clone());
        checkpoint.review = report.review.clone();
        report.gap = None;
        return Ok(());
    }
}

fn validate_identity<'a>(
    report: &'a RunReport,
    checkpoint: &RunCheckpoint,
    identity: &recovery::CallIdentity,
    index: usize,
) -> Result<&'a agent_jobs::Attempt, ClewError> {
    let attempt = report
        .attempts
        .get(index)
        .ok_or_else(|| invalid("RECOVERY_REPORT_MISMATCH: reviewer attempt missing"))?;
    if identity.run != report.run
        || identity.work != report.work
        || identity.role != "reviewer"
        || identity.snapshot != checkpoint.snapshot
        || Some(identity.config_digest.as_str()) != report.config_digest.as_deref()
        || identity.config_digest != checkpoint.config_digest
        || checkpoint.driver_digests.get("reviewer") != Some(&identity.driver_digest)
        || attempt.role != "reviewer"
        || attempt.invocation != identity.invocation
        || attempt.input_digest != identity.input_digest
        || attempt.reservation != identity.reservation
        || attempt.model != identity.model
        || attempt.usage_authority != identity.usage_authority
        || attempt.admission["driverDigest"] != identity.driver_digest
    {
        return Err(invalid(
            "RECOVERY_REPORT_MISMATCH: ordered reviewer attempt differs from its admitted durable invocation",
        ));
    }
    Ok(attempt)
}

fn validate_attempt(
    report: &RunReport,
    checkpoint: &RunCheckpoint,
    identity: &recovery::CallIdentity,
    result_digest: &str,
    index: usize,
) -> Result<(), ClewError> {
    let attempt = validate_identity(report, checkpoint, identity, index)?;
    if attempt.status != "COMPLETED" || attempt.result_digest.as_deref() != Some(result_digest) {
        return Err(invalid(
            "RECOVERY_REPORT_MISMATCH: reviewer attempt differs from its durable result",
        ));
    }
    Ok(())
}

fn validate_result_metadata(
    attempt: &agent_jobs::Attempt,
    saved: &recovery::SavedResult,
) -> Result<(), ClewError> {
    if attempt.result_digest.as_deref() != Some(saved.result_digest.as_str())
        || attempt.usage != saved.usage
        || attempt.captured_stdout_bytes != saved.stdout_bytes
        || attempt.captured_stderr_bytes != saved.stderr_bytes
    {
        return Err(invalid(
            "RECOVERY_RESULT_MISMATCH: reviewer usage or captured stream sizes differ from the immutable result",
        ));
    }
    Ok(())
}

fn validate_selected_suffix(
    material: &Material<'_>,
    selected: &[Selection],
    checkpoint: &RunCheckpoint,
    before_pages: usize,
    before_parts: usize,
    after_pages: usize,
    after_parts: usize,
) -> Result<(), ClewError> {
    let read = work::read_state(material.repo, &material.work.id)?;
    let mut native = Vec::new();
    let mut sources = BTreeSet::new();
    for selection in selected {
        for requested in native_selections(selection)? {
            let effective = recorded_expansion::effective_expansion_selection(&requested);
            if let Ok(rows) = work::audit_rows(material.work, &effective) {
                sources.extend(
                    rows.into_iter()
                        .filter(|row| row["kind"] == "SOURCE")
                        .filter_map(|row| row["reference"].as_str().map(str::to_owned)),
                );
            }
            native.push(effective);
        }
    }
    for page in &checkpoint.pages[before_pages..after_pages] {
        let receipt = read
            .receipts
            .values()
            .find(|receipt| page["receiptDigest"].as_str() == Some(receipt.result_digest.as_str()))
            .ok_or_else(|| {
                invalid("RECOVERY_CONTEXT_MISMATCH: reviewer page has no exact receipt")
            })?;
        if !native.iter().any(|selection| {
            let mut page_selection = receipt.selection.clone();
            let mut requested = selection.clone();
            // Query navigation has one page per explicit cursor; content pages
            // continue internally with that native selection's frozen membership.
            if requested
                .query
                .as_ref()
                .is_some_and(|query| query.kind == "SYMBOL")
            {
                return requested == page_selection;
            }
            requested.cursor = None;
            page_selection.cursor = None;
            requested == page_selection
        }) {
            return Err(invalid(
                "RECOVERY_CONTEXT_MISMATCH: reviewer pages do not belong to the saved grouped request",
            ));
        }
    }
    if checkpoint.source_parts[before_parts..after_parts]
        .iter()
        .any(|part| {
            part["reference"]
                .as_str()
                .is_none_or(|reference| !sources.contains(reference))
        })
    {
        return Err(invalid(
            "RECOVERY_CONTEXT_MISMATCH: reviewer source prefix does not belong to the saved grouped request",
        ));
    }
    crate::documentation::work_parts::validate_source_prefixes(
        material.work,
        &read,
        &checkpoint.source_parts[..after_parts],
    )?;
    let complete_references: BTreeSet<_> = checkpoint.source_parts[..after_parts]
        .iter()
        .filter(|part| part["nextCursor"].is_null())
        .filter_map(|part| part["reference"].as_str())
        .collect();
    let complete_parts: Vec<_> = checkpoint.source_parts[..after_parts]
        .iter()
        .filter(|part| {
            part["reference"]
                .as_str()
                .is_some_and(|reference| complete_references.contains(reference))
        })
        .cloned()
        .collect();
    operation_context::context(
        material.repo,
        material.work,
        &checkpoint.pages[..after_pages],
        &complete_parts,
    )?;
    Ok(())
}

fn validate_complete_delivery(
    material: &Material<'_>,
    selected: &[Selection],
    checkpoint: &RunCheckpoint,
    after_pages: usize,
    after_parts: usize,
) -> Result<(), ClewError> {
    let read = work::read_state(material.repo, &material.work.id)?;
    let complete_sources: BTreeSet<_> = checkpoint.source_parts[..after_parts]
        .iter()
        .filter(|part| part["nextCursor"].is_null())
        .filter_map(|part| part["reference"].as_str())
        .collect();
    for grouped in selected {
        for requested in native_selections(grouped)? {
            let mut effective = recorded_expansion::effective_expansion_selection(&requested);
            if let Err(error) = work::audit_rows(material.work, &effective) {
                if effective.cursor.is_none()
                    && recorded_expansion::symbol_lookup_feedback(
                        &json!({"action":"expand","selection":requested}),
                        &effective,
                        &error,
                    )
                    .is_some()
                {
                    continue;
                }
                return Err(invalid(
                    "RECOVERY_CONTEXT_MISMATCH: completed grouped selection cannot be read from frozen Work",
                ));
            }
            let navigation = effective
                .query
                .as_ref()
                .is_some_and(|query| query.kind == "SYMBOL");
            let mut cursors = BTreeSet::new();
            loop {
                if let Some(cursor) = &effective.cursor
                    && !cursors.insert(cursor.clone())
                {
                    return Err(invalid(
                        "RECOVERY_CONTEXT_MISMATCH: completed delivery cursor repeated",
                    ));
                }
                let page = checkpoint.pages[..after_pages].iter().find(|page| read.receipts.values().any(|receipt|
                    receipt.selection == effective && page["receiptDigest"].as_str() == Some(receipt.result_digest.as_str())))
                    .ok_or_else(|| invalid("RECOVERY_CONTEXT_MISMATCH: completed expansion did not deliver every native content page"))?;
                if page["omitted"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|row| row["kind"] == "SOURCE")
                    .any(|row| {
                        row["reference"]
                            .as_str()
                            .is_none_or(|reference| !complete_sources.contains(reference))
                    })
                {
                    return Err(invalid(
                        "RECOVERY_CONTEXT_MISMATCH: completed expansion did not deliver every omitted source",
                    ));
                }
                if navigation {
                    break;
                }
                effective.cursor = page["nextCursor"].as_str().map(str::to_owned);
                if effective.cursor.is_none() {
                    break;
                }
            }
        }
    }
    Ok(())
}

fn validate_chain(
    material: &Material<'_>,
    report: &RunReport,
    checkpoint: &RunCheckpoint,
    delivery: &DeliveryState,
) -> Result<(), ClewError> {
    if report.execution_mode.as_deref() != Some(MODE)
        || checkpoint.previous != *material.answer
        || checkpoint.feedback != json!(material.blocks)
        || report.draft_review.as_ref() != Some(material.origin)
        || checkpoint.snapshot != material.origin.snapshot
        || report.proposal.is_some()
        || report.draft.is_some()
        || report.draft_repair.is_some()
        || report.publication.as_ref() != Some(&json!({"status":"NOT_PUBLISHED"}))
        || checkpoint.proposal_id.is_some()
        || checkpoint.publication_baseline.is_some()
        || checkpoint.publication_receipt.is_some()
    {
        return Err(invalid(
            "RECOVERY_CONTEXT_MISMATCH: reviewer source, mode or publication state changed",
        ));
    }
    let mut counts = (0, 0);
    let mut preceding_feedback: &[Value] = &[];
    match (&report.draft_review_retry, &delivery.seed) {
        (Some(retry), Some(seed)) => {
            let failed =
                agent_jobs::load_report_by_id(material.repo, &material.work.id, &retry.review_run)?;
            let (expected, prior) = uncertain_source(material, &failed)?;
            let prior_delivery = state(&prior)?;
            if *retry != expected
                || seed.pages != prior.pages.len()
                || seed.parts != prior.source_parts.len()
                || seed.pages > checkpoint.pages.len()
                || seed.parts > checkpoint.source_parts.len()
                || checkpoint.pages[..seed.pages] != prior.pages
                || checkpoint.source_parts[..seed.parts] != prior.source_parts
                || seed.lookup_feedback != last_feedback(&prior_delivery)
                || seed.expansion_feedback != prior.expansion_feedback
            {
                return Err(invalid(
                    "RECOVERY_INPUT_BINDING_MISMATCH: retry seed differs from its exact uncertain review checkpoint",
                ));
            }
            counts = (seed.pages, seed.parts);
            preceding_feedback = &seed.lookup_feedback;
        }
        (None, None) => {}
        _ => {
            return Err(invalid(
                "RECOVERY_INPUT_BINDING_MISMATCH: reviewer retry lineage and frozen evidence seed disagree",
            ));
        }
    }
    for (index, step) in delivery.steps.iter().enumerate() {
        if (step.before_pages, step.before_parts) != counts
            || step.after_pages < step.before_pages
            || step.after_parts < step.before_parts
            || step.after_pages > checkpoint.pages.len()
            || step.after_parts > checkpoint.source_parts.len()
        {
            return Err(invalid(
                "RECOVERY_CONTEXT_MISMATCH: reviewer delivery prefixes changed",
            ));
        }
        validate_attempt(
            report,
            checkpoint,
            &step.identity,
            &step.result_digest,
            index,
        )?;
        let input = recovery::load_input(material.repo, &step.identity)?;
        let result = recovery::load_result(material.repo, &input)?;
        validate_result_metadata(&report.attempts[index], &result)?;
        if input.request["payload"]
            != material.payload(
                &checkpoint.pages[..step.before_pages],
                &checkpoint.source_parts[..step.before_parts],
                preceding_feedback,
                delivery.configured_calls,
                index,
            )?
            || result.result_digest != step.result_digest
        {
            return Err(invalid(
                "RECOVERY_INPUT_BINDING_MISMATCH: recorded reviewer expansion changed",
            ));
        }
        let selected = selections(&result.result).map_err(|_| {
            invalid("RECOVERY_RESULT_MISMATCH: completed expansion action is invalid")
        })?;
        validate_selected_suffix(
            material,
            &selected,
            checkpoint,
            step.before_pages,
            step.before_parts,
            step.after_pages,
            step.after_parts,
        )?;
        if step.lookup_feedback != feedback(material.work, &selected)? {
            return Err(invalid(
                "RECOVERY_CONTEXT_MISMATCH: saved lookup feedback differs from frozen Work",
            ));
        }
        // Every completed role decision has fully delivered citable context.
        operation_context::context(
            material.repo,
            material.work,
            &checkpoint.pages[..step.after_pages],
            &checkpoint.source_parts[..step.after_parts],
        )?;
        validate_complete_delivery(
            material,
            &selected,
            checkpoint,
            step.after_pages,
            step.after_parts,
        )?;
        counts = (step.after_pages, step.after_parts);
        preceding_feedback = &step.lookup_feedback;
    }
    if let Some(pending) = &delivery.pending {
        if (pending.before_pages, pending.before_parts) != counts
            || pending.before_pages > checkpoint.pages.len()
            || pending.before_parts > checkpoint.source_parts.len()
        {
            return Err(invalid(
                "RECOVERY_CONTEXT_MISMATCH: pending reviewer read prefix changed",
            ));
        }
        validate_attempt(
            report,
            checkpoint,
            &pending.identity,
            &pending.result_digest,
            delivery.steps.len(),
        )?;
        let input = recovery::load_input(material.repo, &pending.identity)?;
        let result = recovery::load_result(material.repo, &input)?;
        validate_result_metadata(&report.attempts[delivery.steps.len()], &result)?;
        if input.request["payload"]
            != material.payload(
                &checkpoint.pages[..pending.before_pages],
                &checkpoint.source_parts[..pending.before_parts],
                preceding_feedback,
                delivery.configured_calls,
                delivery.steps.len(),
            )?
            || result.result_digest != pending.result_digest
            || selections(&result.result).map_err(|_| {
                invalid("RECOVERY_RESULT_MISMATCH: pending grouped request is invalid")
            })? != pending.selections
            || checkpoint.pending_call.as_ref().is_none_or(|call| {
                call.identity != pending.identity || call.status != "RESULT_SAVED"
            })
        {
            return Err(invalid(
                "RECOVERY_INPUT_BINDING_MISMATCH: pending reviewer read changed",
            ));
        }
        validate_selected_suffix(
            material,
            &pending.selections,
            checkpoint,
            pending.before_pages,
            pending.before_parts,
            checkpoint.pages.len(),
            checkpoint.source_parts.len(),
        )?;
    } else if counts != (checkpoint.pages.len(), checkpoint.source_parts.len()) {
        return Err(invalid(
            "RECOVERY_CONTEXT_MISMATCH: reviewer has unbound evidence",
        ));
    }
    let expected_calls = delivery.steps.len() + usize::from(checkpoint.pending_call.is_some());
    if delivery.configured_calls == 0
        || expected_calls > delivery.configured_calls as usize
        || report.attempts.len() != expected_calls
        || report
            .attempts
            .iter()
            .map(|attempt| &attempt.invocation)
            .collect::<BTreeSet<_>>()
            .len()
            != report.attempts.len()
        || report
            .attempts
            .iter()
            .map(|attempt| &attempt.reservation)
            .collect::<BTreeSet<_>>()
            .len()
            != report.attempts.len()
    {
        return Err(invalid(
            "RECOVERY_REPORT_MISMATCH: reviewer attempt chain is incomplete or duplicated",
        ));
    }
    if let Some(pending) = &checkpoint.pending_call {
        let attempt =
            validate_identity(report, checkpoint, &pending.identity, delivery.steps.len())?;
        let input = recovery::load_input(material.repo, &pending.identity)?;
        if let Some(saved) = recovery::try_load_result(material.repo, &input)? {
            validate_result_metadata(attempt, &saved)?;
        }
    }
    let expected_feedback = delivery
        .steps
        .last()
        .map(|step| json!({"lookupFeedback":step.lookup_feedback}))
        .or_else(|| {
            delivery
                .seed
                .as_ref()
                .and_then(|seed| seed.expansion_feedback.clone())
        });
    if checkpoint.expansion_feedback != expected_feedback {
        return Err(invalid(
            "RECOVERY_CONTEXT_MISMATCH: delivered lookup feedback changed",
        ));
    }
    if checkpoint.phase != "TERMINAL"
        && delivery.pending.is_none()
        && checkpoint.read_digest != digest(&work::read_state(material.repo, &material.work.id)?)?
    {
        return Err(invalid(
            "RECOVERY_CONTEXT_MISMATCH: reviewer read state changed since its selected phase",
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn validate_saved_payload(
    repo: &Repository,
    work: &Work,
    origin: &Origin,
    packet: &Value,
    answer: &Value,
    blocks: &[Value],
    author: &Value,
    report: &RunReport,
    checkpoint: &RunCheckpoint,
    payload: &Value,
) -> Result<(), ClewError> {
    let material = Material {
        repo,
        work,
        origin,
        packet,
        answer,
        blocks,
        author,
    };
    let delivery = state(checkpoint)?;
    validate_chain(&material, report, checkpoint, &delivery)?;
    if delivery.pending.is_some()
        || *payload
            != material.payload(
                &checkpoint.pages,
                &checkpoint.source_parts,
                last_feedback(&delivery),
                delivery.configured_calls,
                delivery.steps.len(),
            )?
    {
        return Err(invalid(
            "RECOVERY_INPUT_BINDING_MISMATCH: final reviewer context differs",
        ));
    }
    Ok(())
}

fn uncertain_source(
    material: &Material<'_>,
    failed: &RunReport,
) -> Result<(RetryOrigin, RunCheckpoint), ClewError> {
    if failed.accounting.is_none() {
        return Err(invalid(
            "DRAFT_REVIEW_RETRY_ACCOUNTING_INCOMPLETE: replay the original uncertain review with its original config to finish retained accounting before an explicit retry",
        ));
    }
    if failed.execution_mode.as_deref() != Some(MODE)
        || failed.status != "DRAFT_REVIEW_UNCERTAIN"
        || failed.draft_review_retry.is_some()
        || failed.review.is_some()
    {
        return Err(invalid(
            "DRAFT_REVIEW_RETRY_INELIGIBLE: select one original terminal uncertain expanding review without a saved result; completed, invalid and retry children cannot be replaced",
        ));
    }
    let reference = failed.checkpoint.as_ref().ok_or_else(|| {
        invalid("RECOVERY_CHECKPOINT_MISSING: uncertain review has no selected checkpoint")
    })?;
    let checkpoint: RunCheckpoint = recovery::load_checkpoint(material.repo, reference)?;
    let config_digest = failed.config_digest.as_deref().ok_or_else(|| {
        invalid("RECOVERY_CONFIG_MISMATCH: uncertain review has no config binding")
    })?;
    checkpoint.validate(failed, config_digest, &checkpoint.driver_digests)?;
    if checkpoint.phase != "TERMINAL" {
        return Err(invalid(
            "DRAFT_REVIEW_RETRY_INELIGIBLE: uncertain source is not terminal",
        ));
    }
    let delivery = state(&checkpoint)?;
    if delivery.pending.is_some() {
        return Err(invalid(
            "DRAFT_REVIEW_RETRY_INELIGIBLE: a saved expansion result is not an uncertain dispatch",
        ));
    }
    validate_chain(material, failed, &checkpoint, &delivery)?;
    validate_terminal(material, failed, &checkpoint)?;
    let pending = checkpoint.pending_call.as_ref().ok_or_else(|| {
        invalid("RECOVERY_REPORT_MISMATCH: uncertain review has no exact invocation")
    })?;
    let attempt = validate_identity(failed, &checkpoint, &pending.identity, delivery.steps.len())?;
    let input = recovery::load_input(material.repo, &pending.identity)?;
    if !matches!(pending.status.as_str(), "FAILED" | "UNCERTAIN_NO_RESULT")
        || !matches!(
            attempt.status.as_str(),
            "FAILED" | "DISPATCH_UNCERTAIN_MAXIMUM_RETAINED"
        )
        || attempt.result_digest.is_some()
        || recovery::try_load_result(material.repo, &input)?.is_some()
    {
        return Err(invalid(
            "DRAFT_REVIEW_RETRY_INELIGIBLE: uncertain invocation has a durable result or different dispatch state",
        ));
    }
    Ok((
        RetryOrigin {
            schema: "codeclew-operation-draft-review-retry/1.0".into(),
            review_run: failed.run.clone(),
            review_checkpoint: reference.clone(),
            review_invocation: pending.identity.invocation.clone(),
            review_input_digest: pending.identity.input_digest.clone(),
        },
        checkpoint,
    ))
}

/// Retry lineage binds the original uncertain invocation and its fully delivered
/// evidence. A child's later final payload legitimately contains more evidence.
pub(super) fn validate_retry_lineage(
    repo: &Repository,
    work: &Work,
    retry: &RetryOrigin,
    origin: &Origin,
    answer: &Value,
    blocks: &[Value],
) -> Result<(), ClewError> {
    super::super::operation_draft::validate_run_id(&retry.review_run)?;
    let source = agent_jobs::load_report_by_id(repo, &work.id, &origin.source_run)?;
    let (packet, audit) = super::super::operation_draft::review_packet(repo, work, &source)?;
    let (expected_origin, expected_answer, author) =
        super::super::operation_draft::review_source(repo, work, &source, &packet, &audit)?;
    if *origin != expected_origin
        || *answer != expected_answer
        || blocks != crate::documentation::operation_answer::review_blocks(answer)?
    {
        return Err(invalid(
            "RECOVERY_INPUT_BINDING_MISMATCH: retry source author binding changed",
        ));
    }
    let material = Material {
        repo,
        work,
        origin,
        packet: &packet,
        answer,
        blocks,
        author: &author,
    };
    let failed = agent_jobs::load_report_by_id(repo, &work.id, &retry.review_run)?;
    let (expected, _) = uncertain_source(&material, &failed)?;
    if *retry != expected {
        return Err(invalid(
            "RECOVERY_INPUT_BINDING_MISMATCH: retry lineage differs from its exact uncertain invocation",
        ));
    }
    Ok(())
}

fn validate_terminal(
    material: &Material<'_>,
    report: &RunReport,
    checkpoint: &RunCheckpoint,
) -> Result<(), ClewError> {
    let delivery = state(checkpoint)?;
    if let Some(pending) = &checkpoint.pending_call {
        let input = recovery::load_input(material.repo, &pending.identity)?;
        let (pages, parts) = delivery.pending.as_ref().map_or(
            (checkpoint.pages.len(), checkpoint.source_parts.len()),
            |expansion| (expansion.before_pages, expansion.before_parts),
        );
        if input.request["payload"]
            != material.payload(
                &checkpoint.pages[..pages],
                &checkpoint.source_parts[..parts],
                last_feedback(&delivery),
                delivery.configured_calls,
                delivery.steps.len(),
            )?
        {
            return Err(invalid(
                "RECOVERY_INPUT_BINDING_MISMATCH: terminal reviewer request changed",
            ));
        }
        if pending.status == "RESULT_SAVED" {
            let result = recovery::load_result(material.repo, &input)?;
            validate_attempt(
                report,
                checkpoint,
                &pending.identity,
                &result.result_digest,
                delivery.steps.len(),
            )?;
            if delivery.pending.is_none() && result.result["action"] == "review" {
                let context = &input.request["payload"]["reviewContext"];
                match validate_action(&result.result, "review", "review").and_then(|_| {
                    validate_context_review(
                        result.result["review"].clone(),
                        material.work,
                        material.origin,
                        material.packet,
                        material.blocks,
                        context,
                    )
                }) {
                    Ok(review)
                        if report.status == verdict_status(&review.verdict)
                            && report.review.as_ref() == Some(&result.result["review"])
                            && checkpoint.review == report.review =>
                    {
                        return Ok(());
                    }
                    Err(_)
                        if report.status == "DRAFT_REVIEW_INVALID_RESULT"
                            && report.review.is_none()
                            && checkpoint.review.is_none() =>
                    {
                        return Ok(());
                    }
                    _ => {}
                }
            } else if delivery.pending.is_none()
                && report.status == "DRAFT_REVIEW_INVALID_RESULT"
                && report.review.is_none()
                && checkpoint.review.is_none()
                && (result.result["action"] != "expand" || selections(&result.result).is_err())
            {
                return Ok(());
            }
            if report.review.is_none()
                && checkpoint.review.is_none()
                && matches!(
                    report.status.as_str(),
                    "DRAFT_REVIEW_FAILED" | "DRAFT_REVIEW_CANCELLED"
                )
            {
                return Ok(());
            }
            return Err(invalid(
                "RECOVERY_RESULT_MISMATCH: terminal reviewer result changed",
            ));
        }
        let attempt =
            validate_identity(report, checkpoint, &pending.identity, delivery.steps.len())?;
        if let Some(saved) = recovery::try_load_result(material.repo, &input)? {
            if pending.status == "FAILED"
                && attempt.status == "FAILED"
                && pending
                    .failure
                    .as_deref()
                    .is_some_and(|failure| failure.starts_with("ACCOUNTING_BOUND_VIOLATED"))
                && attempt.failure == pending.failure
                && attempt.result_digest.as_deref() == Some(saved.result_digest.as_str())
                && attempt.usage == saved.usage
                && attempt.captured_stdout_bytes == saved.stdout_bytes
                && attempt.captured_stderr_bytes == saved.stderr_bytes
                && report.status == "DRAFT_REVIEW_FAILED"
                && report.review.is_none()
                && checkpoint.review.is_none()
            {
                return Ok(());
            }
            return Err(invalid(
                "RECOVERY_RESULT_MISMATCH: unadmitted durable reviewer result differs from its exact accounting failure",
            ));
        }
        if attempt.result_digest.is_some()
            || !matches!(
                pending.status.as_str(),
                "FAILED" | "UNCERTAIN_NO_RESULT" | "PREPARED"
            )
        {
            return Err(invalid(
                "RECOVERY_RESULT_MISMATCH: terminal reviewer dispatch state changed",
            ));
        }
    }
    if report.review.is_some()
        || checkpoint.review.is_some()
        || !matches!(
            report.status.as_str(),
            "DRAFT_REVIEW_FAILED"
                | "DRAFT_REVIEW_CANCELLED"
                | "DRAFT_REVIEW_UNCERTAIN"
                | "DRAFT_REVIEW_NEEDS_EVIDENCE"
        )
    {
        return Err(invalid(
            "RECOVERY_RESULT_MISMATCH: terminal review has no matching durable verdict or failure",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::PathBuf};

    const DRIVER: &str = r#"
r = JSON.parse(STDIN.read)
p = r.fetch("payload")
mode = ARGV.fetch(0)
exit 7 if mode == "uncertain"
exit 7 if mode == "uncertain-after-expand" && !p.fetch("reviewContext").fetch("pages").empty?
refs = (0...19).map { |i| "expanded-ref-%02d" % i }
action = if mode == "empty"
  {"action" => "expand", "selections" => []}
elsif mode == "unknown"
  {"action" => "invented"}
elsif mode == "foreign"
  {"action" => "expand", "selections" => [{"references" => ["FOREIGN"]}]}
elsif mode == "untracked"
  {"action" => "expand", "selections" => [{"references" => refs, "untrackedReads" => true}]}
elsif mode == "feedback" && p.fetch("reviewContext").fetch("pages").empty? && p.fetch("lookupFeedback").empty?
  {"action" => "expand", "selections" => [{"symbols" => ["missing-review-only-symbol"]}]}
elsif p.fetch("reviewContext").fetch("pages").empty?
  if mode == "feedback"
    abort "missing bounded lookup feedback" unless p.fetch("lookupFeedback").first.fetch("status") == "NOT_FOUND"
  end
  {"action" => "expand", "selections" => [{"references" => refs}]}
else
  s = p.fetch("outputSchema").fetch("$defs").fetch("review").fetch("properties")
  review = %w[schema work sourceRun sourceInvocation snapshot packetDigest answerDigest coverageDigest reviewContextDigest].to_h { |k| [k, s.fetch(k).fetch("const")] }
  review["verdict"] = "APPROVE"
  review["assessedBlocks"] = p.fetch("blocks").map { |b| b.fetch("id") }
  review["assessedEvidence"] = p.fetch("blocks").flat_map { |b| b.fetch("evidence") }.uniq.sort
  review["issues"] = []
  review["limitations"] = ["Synthetic independent source review; no runtime or deployment claim."]
  {"action" => "review", "review" => review}
end
reply = {"schema" => "codeclew-documentation-agent-result/1.0", "invocation" => r.fetch("invocation"), "role" => r.fetch("role"), "model" => r.fetch("model"), "result" => action}
reply["usage"] = {"inputTokens" => 600001, "outputTokens" => 1, "costUnits" => 1} if mode == "overcap"
puts JSON.generate(reply)
"#;

    fn config(path: &Path, author: &Path, mode: &str, calls: u32) -> ContextConfig {
        let original: Value = store::read(author, store::MAX_RECORD).unwrap();
        let mut reviewer = original["author"].clone();
        reviewer["model"] = json!(format!("synthetic-independent-review-{mode}"));
        reviewer["command"] = json!([
            "/usr/bin/ruby",
            "--disable-gems",
            "-rjson",
            "-e",
            DRIVER,
            mode
        ]);
        reviewer["cap"]["maximum"]["inputTokens"] = json!(600000);
        if mode == "overcap" {
            reviewer["usageAuthority"] = json!("TRANSPORT_METADATA");
        }
        let value = json!({"schema":CONFIG_SCHEMA,"reviewer":reviewer,"reviewerCalls":calls,
            "budget":{"account":format!("independent-review-{mode}"),"costUnit":"local-reservation-unit",
                "ceiling":{"inputTokens":3000000,"outputTokens":100000,"costUnits":100},
                "stopLoss":{"inputTokens":2100000,"outputTokens":75000,"costUnits":60}}});
        fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
        serde_json::from_value(value).unwrap()
    }

    fn files(path: &Path, result: &mut BTreeMap<PathBuf, Vec<u8>>) {
        if path.is_dir() {
            for entry in fs::read_dir(path).unwrap() {
                files(&entry.unwrap().path(), result);
            }
        } else if path.is_file() {
            result.insert(path.to_owned(), fs::read(path).unwrap());
        }
    }

    fn author_files(
        repo: &Repository,
        work: &Work,
        author: &Path,
        run: &str,
    ) -> BTreeMap<PathBuf, Vec<u8>> {
        let original: Value = store::read(author, store::MAX_RECORD).unwrap();
        let source = agent_jobs::load_report_by_id(repo, &work.id, run).unwrap();
        let mut result = BTreeMap::new();
        for relative in [
            format!(".codeclew/jobs/{run}.json"),
            format!(".codeclew/jobs/{run}"),
            format!(".codeclew/drafts/{}", work.id),
            format!(
                "execution/accounts/{}.json",
                original["budget"]["account"].as_str().unwrap()
            ),
        ] {
            files(&repo.path(&relative).unwrap(), &mut result);
        }
        for attempt in &source.attempts {
            for folder in ["job-inputs", "job-results"] {
                files(
                    &repo
                        .path(&format!(".codeclew/{folder}/{}.json", attempt.invocation))
                        .unwrap(),
                    &mut result,
                );
            }
        }
        result
    }

    fn checkpoint(repo: &Repository, report: &RunReport, cfg: &ContextConfig) -> RunCheckpoint {
        let admission = crate::documentation::agent_adapter::admit(repo, &cfg.reviewer).unwrap();
        let drivers = BTreeMap::from([(
            "reviewer".into(),
            admission["driverDigest"].as_str().unwrap().into(),
        )]);
        agent_jobs::load_run_checkpoint(repo, report, &digest(cfg).unwrap(), &drivers)
            .unwrap()
            .unwrap()
    }

    #[test]
    fn expanding_reviewer_explicit_uncertain_retry_freezes_delivered_seed_and_preserves_failed_account()
     {
        if !cfg!(target_os = "macos") {
            return;
        }
        let (temp, repo, work, author, source) =
            super::super::super::operation_draft::tests::authored_context();
        let failed_path = temp.path().join("failed-review.json");
        let failed_cfg = config(&failed_path, &author, "uncertain-after-expand", 3);
        let failed = run(&repo, &work, &source, &failed_path, None).unwrap();
        assert_eq!(failed["status"], "DRAFT_REVIEW_UNCERTAIN", "{failed}");
        let failed_report = agent_jobs::latest_report(&repo, &work.id).unwrap().unwrap();
        assert_eq!(failed_report.attempts.len(), 2);
        let failed_cp = checkpoint(&repo, &failed_report, &failed_cfg);
        assert!(!failed_cp.pages.is_empty());
        assert!(!failed_cp.source_parts.is_empty());
        let mut originals = author_files(&repo, &work, &author, &source);
        for relative in [
            format!(".codeclew/jobs/{}.json", failed_report.run),
            format!(".codeclew/jobs/{}", failed_report.run),
            format!("execution/accounts/{}.json", failed_cfg.budget.account),
        ] {
            files(&repo.path(&relative).unwrap(), &mut originals);
        }
        for attempt in &failed_report.attempts {
            for folder in ["job-inputs", "job-results"] {
                files(
                    &repo
                        .path(&format!(".codeclew/{folder}/{}.json", attempt.invocation))
                        .unwrap(),
                    &mut originals,
                );
            }
        }
        let path = temp.path().join("retry-review.json");
        let cfg = config(&path, &author, "approve", 2);
        let mut shared: Value = store::read(&path, store::MAX_RECORD).unwrap();
        shared["budget"]["account"] = json!(failed_cfg.budget.account);
        let shared_path = temp.path().join("shared-review.json");
        fs::write(&shared_path, serde_json::to_vec(&shared).unwrap()).unwrap();
        assert!(
            run(
                &repo,
                &work,
                &source,
                &shared_path,
                Some(&failed_report.run)
            )
            .unwrap_err()
            .message
            .contains("ACCOUNT_CONFLICT")
        );
        let approved = run(&repo, &work, &source, &path, Some(&failed_report.run)).unwrap();
        assert_eq!(approved["status"], "DRAFT_REVIEW_APPROVED", "{approved}");
        let child = agent_jobs::latest_report(&repo, &work.id).unwrap().unwrap();
        assert_ne!(child.run, failed_report.run);
        assert_eq!(child.attempts.len(), 1);
        assert_eq!(
            child.draft_review_retry.as_ref().unwrap().review_run,
            failed_report.run
        );
        let cp = checkpoint(&repo, &child, &cfg);
        let delivery = state(&cp).unwrap();
        assert!(delivery.steps.is_empty());
        assert_eq!(delivery.seed.as_ref().unwrap().pages, failed_cp.pages.len());
        assert_eq!(cp.pages, failed_cp.pages);
        assert_eq!(cp.source_parts, failed_cp.source_parts);
        let selected = super::super::load_approved_answer(&repo, &work, &child.run).unwrap();
        assert_eq!(
            selected.provenance["reviewRetry"],
            json!(child.draft_review_retry)
        );
        super::super::export_approved_answer(
            &repo,
            &work.id,
            &child.run,
            &temp.path().join("retry-export"),
        )
        .unwrap();
        let charged = json!(account(&repo, &cfg.budget).unwrap());
        assert_eq!(run(&repo, &work, &source, &path, None).unwrap(), approved);
        assert_eq!(json!(account(&repo, &cfg.budget).unwrap()), charged);
        assert!(
            run(&repo, &work, &source, &path, Some(&failed_report.run))
                .unwrap_err()
                .message
                .contains("ALREADY_FINISHED")
        );
        assert!(
            run(&repo, &work, &source, &path, Some(&child.run))
                .unwrap_err()
                .message
                .contains("INELIGIBLE")
        );
        for (path, bytes) in originals {
            assert_eq!(fs::read(path).unwrap(), bytes);
        }
    }

    #[test]
    fn expanding_reviewer_configured_budget_and_initial_envelope_refuse_before_allocation() {
        if !cfg!(target_os = "macos") {
            return;
        }
        let (temp, repo, work, author, source) =
            super::super::super::operation_draft::tests::authored_context();
        let path = temp.path().join("review.json");
        let mut cfg = config(&path, &author, "approve", 4);
        assert!(
            run(&repo, &work, &source, &path, None)
                .unwrap_err()
                .message
                .contains("BUDGET_EXHAUSTED")
        );
        let account_path = repo
            .path(&format!("execution/accounts/{}.json", cfg.budget.account))
            .unwrap();
        assert!(!account_path.exists());
        assert_eq!(
            agent_jobs::latest_report(&repo, &work.id)
                .unwrap()
                .unwrap()
                .run,
            source
        );
        cfg.reviewer_calls = 2;
        cfg.reviewer.cap.maximum.input_tokens = 1;
        fs::write(&path, serde_json::to_vec(&cfg).unwrap()).unwrap();
        assert!(
            run(&repo, &work, &source, &path, None)
                .unwrap_err()
                .message
                .contains("INPUT")
        );
        assert!(!account_path.exists());
        assert_eq!(
            agent_jobs::latest_report(&repo, &work.id)
                .unwrap()
                .unwrap()
                .run,
            source
        );
        cfg.reviewer_calls = 65;
        cfg.budget.stop_loss.cost_units = 90;
        cfg.reviewer.cap.maximum = agent_jobs::Amount {
            input_tokens: 1,
            output_tokens: 1,
            cost_units: 1,
        };
        feasible_budget(&cfg, &account(&repo, &cfg.budget).unwrap(), "fresh").unwrap();
        cfg.reviewer.cap.maximum.input_tokens = u64::MAX;
        assert!(
            feasible_budget(&cfg, &account(&repo, &cfg.budget).unwrap(), "fresh")
                .unwrap_err()
                .message
                .contains("overflow")
        );
    }

    #[test]
    fn expanding_reviewer_resumes_sparse_group_after_durable_source_part_without_duplicate_reads() {
        if !cfg!(target_os = "macos") {
            return;
        }
        let (temp, repo, work, author, source_run) =
            super::super::super::operation_draft::tests::authored_context();
        let path = temp.path().join("review.json");
        let cfg = config(&path, &author, "approve", 3);
        let frozen = author_files(&repo, &work, &author, &source_run);
        let source = agent_jobs::load_report_by_id(&repo, &work.id, &source_run).unwrap();
        let (packet, audit) =
            super::super::super::operation_draft::review_packet(&repo, &work, &source).unwrap();
        let (origin, answer, author_contract) =
            super::super::super::operation_draft::review_source(
                &repo, &work, &source, &packet, &audit,
            )
            .unwrap();
        let blocks = crate::documentation::operation_answer::review_blocks(&answer).unwrap();
        let material = Material {
            repo: &repo,
            work: &work,
            origin: &origin,
            packet: &packet,
            answer: &answer,
            blocks: &blocks,
            author: &author_contract,
        };
        let config = Config {
            schema: "codeclew-documentation-execution/1.0".into(),
            author: cfg.reviewer.clone(),
            reviewer: cfg.reviewer.clone(),
            author_output_contract: None,
            fallback: None,
            author_calls: 0,
            reviewer_calls: cfg.reviewer_calls,
            fallback_calls: 0,
            repair_attempts: 0,
            expansions: cfg.reviewer_calls - 1,
            budget: cfg.budget.clone(),
        };
        let admission = crate::documentation::agent_adapter::admit(&repo, &cfg.reviewer).unwrap();
        let drivers = BTreeMap::from([(
            "reviewer".into(),
            admission["driverDigest"].as_str().unwrap().into(),
        )]);
        let mut report = RunReport {
            schema: "codeclew-documentation-work-run/1.0".into(),
            run: uuid::Uuid::new_v4().simple().to_string(),
            work: work.id.clone(),
            status: "PREPARED".into(),
            config_digest: Some(digest(&cfg).unwrap()),
            attempts: vec![],
            proposal: None,
            review: None,
            publication: Some(json!({"status":"NOT_PUBLISHED"})),
            gap: None,
            accounting: None,
            context_budget: None,
            execution_mode: Some(MODE.into()),
            draft: None,
            draft_repair: None,
            draft_review: Some(origin.clone()),
            draft_review_retry: None,
            checkpoint: None,
        };
        let mut cp = RunCheckpoint::new(
            &report,
            origin.snapshot.clone(),
            digest(&cfg).unwrap(),
            drivers,
            digest(&work::read_state(&repo, &work.id).unwrap()).unwrap(),
        );
        cp.phase = "REVIEWER".into();
        cp.previous = answer.clone();
        cp.feedback = json!(blocks);
        let mut delivery = DeliveryState {
            configured_calls: cfg.reviewer_calls,
            ..DeliveryState::default()
        };
        set_state(&mut cp, &delivery).unwrap();
        agent_jobs::ensure_reserved(&repo, &config, &report.run).unwrap();
        agent_jobs::save_run_checkpoint(&repo, &mut report, &cp).unwrap();
        let (raw, _, _) = agent_jobs::call(
            &repo,
            &config,
            &mut report,
            &mut cp,
            "reviewer",
            &cfg.reviewer,
            material
                .payload(&[], &[], &[], cfg.reviewer_calls, 0)
                .unwrap(),
            None,
            false,
            false,
        )
        .unwrap();
        assert!(raw["selections"][0].get("symbols").is_none());
        let selected = selections(&raw).unwrap();
        delivery.pending = Some(PendingExpansion {
            identity: cp.pending_call.as_ref().unwrap().identity.clone(),
            selections: selected.clone(),
            before_pages: 0,
            before_parts: 0,
            result_digest: report.attempts[0].result_digest.clone().unwrap(),
        });
        set_state(&mut cp, &delivery).unwrap();
        agent_jobs::save_run_checkpoint(&repo, &mut report, &cp).unwrap();
        let mut pages = vec![];
        let mut parts = vec![];
        let interrupted = recorded_expansion::retrieve(
            &repo,
            &work,
            &selected,
            &mut pages,
            &mut parts,
            |stage, saved_pages, saved_parts| {
                cp.pages = saved_pages.to_vec();
                cp.source_parts = saved_parts.to_vec();
                cp.read_digest = digest(&work::read_state(&repo, &work.id)?)?;
                agent_jobs::save_run_checkpoint(&repo, &mut report, &cp)?;
                if matches!(stage, recorded_expansion::ProgressStage::SourcePart) {
                    return Err(invalid(
                        "synthetic process interruption after durable source prefix",
                    ));
                }
                Ok(())
            },
        )
        .unwrap_err();
        assert!(
            interrupted
                .message
                .contains("synthetic process interruption")
        );
        assert_eq!(cp.source_parts.len(), 1);
        assert!(!cp.source_parts[0]["nextCursor"].is_null());
        let first = cp.source_parts[0].clone();
        let read_before = json!(work::read_state(&repo, &work.id).unwrap());
        let saved = cp.clone();
        cp.source_parts[0]["text"] = json!("forged source text");
        agent_jobs::save_run_checkpoint(&repo, &mut report, &cp).unwrap();
        let charged = json!(account(&repo, &cfg.budget).unwrap());
        assert!(run(&repo, &work, &source_run, &path, None).is_err());
        assert_eq!(json!(account(&repo, &cfg.budget).unwrap()), charged);
        assert_eq!(
            json!(work::read_state(&repo, &work.id).unwrap()),
            read_before
        );
        agent_jobs::save_run_checkpoint(&repo, &mut report, &saved).unwrap();
        let result = run(&repo, &work, &source_run, &path, None).unwrap();
        assert_eq!(result["status"], "DRAFT_REVIEW_APPROVED", "{result}");
        let final_report = agent_jobs::latest_report(&repo, &work.id).unwrap().unwrap();
        assert_eq!(final_report.attempts.len(), 2);
        assert_eq!(
            final_report.attempts[0].invocation,
            report.attempts[0].invocation
        );
        let completed = checkpoint(&repo, &final_report, &cfg);
        assert_eq!(completed.source_parts[0], first);
        assert_eq!(
            completed
                .source_parts
                .iter()
                .filter(|p| **p == first)
                .count(),
            1
        );
        assert_eq!(author_files(&repo, &work, &author, &source_run), frozen);
        let read = json!(work::read_state(&repo, &work.id).unwrap());
        let charged = json!(account(&repo, &cfg.budget).unwrap());
        assert_eq!(run(&repo, &work, &source_run, &path, None).unwrap(), result);
        assert_eq!(json!(work::read_state(&repo, &work.id).unwrap()), read);
        assert_eq!(json!(account(&repo, &cfg.budget).unwrap()), charged);
        super::super::load_approved_answer(&repo, &work, &final_report.run).unwrap();
    }

    #[test]
    fn expanding_reviewer_sparse_batch_feedback_export_binds_every_attempt_and_preserves_author() {
        if !cfg!(target_os = "macos") {
            return;
        }
        let (temp, repo, work, author, source) =
            super::super::super::operation_draft::tests::authored_context();
        let path = temp.path().join("review.json");
        let cfg = config(&path, &author, "feedback", 3);
        let before = author_files(&repo, &work, &author, &source);
        let result = run(&repo, &work, &source, &path, None).unwrap();
        assert_eq!(result["status"], "DRAFT_REVIEW_APPROVED", "{result}");
        let report = agent_jobs::latest_report(&repo, &work.id).unwrap().unwrap();
        assert_eq!(report.attempts.len(), 3);
        let mut cp = checkpoint(&repo, &report, &cfg);
        let delivery = state(&cp).unwrap();
        assert_eq!(delivery.steps.len(), 2);
        assert_eq!(delivery.steps[0].lookup_feedback[0]["status"], "NOT_FOUND");
        assert!(cp.pages.len() > 8);
        assert!(cp.source_parts.len() > 1);
        let second = recovery::load_input(&repo, &delivery.steps[1].identity).unwrap();
        assert_eq!(
            second.request["payload"]["lookupFeedback"],
            json!(delivery.steps[0].lookup_feedback)
        );
        assert_eq!(
            second.request["payload"]["reviewContext"]["pages"],
            json!([])
        );
        let selected = super::super::load_approved_answer(&repo, &work, &report.run).unwrap();
        let terminal =
            recovery::load_input(&repo, &cp.pending_call.as_ref().unwrap().identity).unwrap();
        let wrapper = recovery::try_load_result(&repo, &terminal)
            .unwrap()
            .unwrap();
        assert_eq!(
            second.request["payload"]["roleBudget"],
            json!({"configuredCalls":3,"remainingCalls":2})
        );
        assert_eq!(
            terminal.request["payload"]["roleBudget"],
            json!({"configuredCalls":3,"remainingCalls":1})
        );
        assert_eq!(
            selected.packet["fields"][0]["ownerIdentity"],
            "class:orders.Service"
        );
        assert_eq!(
            selected.packet["fields"][0]["sourceTokens"],
            json!(["static", "final", "String", "DEFAULT_CODE", "=", "default"])
        );
        assert_eq!(selected.provenance["runtime"], "UNKNOWN");
        assert_eq!(
            selected.provenance["reviewerEvidence"]["sourceParts"][0]["source"]["authority"],
            work.checked.services["orders"].sources["expanded-source-0"].authority
        );
        assert_eq!(
            selected.provenance["reviewerEvidence"]["snapshot"],
            work.snapshot.clone().unwrap()
        );
        assert!(
            selected.provenance["reviewerEvidence"]["citations"]
                .get("expanded-ref-00")
                .is_some()
        );
        assert_eq!(
            selected.review["reviewContextDigest"],
            terminal.request["payload"]["reviewContext"]["deliveredDigest"]
        );
        assert_eq!(
            selected.provenance["reviewer"]["resultDigest"],
            wrapper.result_digest
        );
        assert_eq!(selected.packet, terminal.request["payload"]["packet"]);
        let output = temp.path().join("approved");
        super::super::export_approved_answer(&repo, &work.id, &report.run, &output).unwrap();
        assert_eq!(author_files(&repo, &work, &author, &source), before);
        let ledger = json!(account(&repo, &cfg.budget).unwrap());
        assert_eq!(run(&repo, &work, &source, &path, None).unwrap(), result);
        assert_eq!(json!(account(&repo, &cfg.budget).unwrap()), ledger);
        let mut forged = report.clone();
        forged.attempts[0].captured_stdout_bytes += 1;
        agent_jobs::save_report(&repo, &forged).unwrap();
        assert!(
            super::super::load_approved_answer(&repo, &work, &report.run)
                .unwrap_err()
                .message
                .contains("immutable result")
        );
        let mut forged = report.clone();
        forged.attempts.last_mut().unwrap().usage = Some(agent_jobs::Usage {
            input_tokens: Some(1),
            output_tokens: None,
            cost_units: None,
        });
        agent_jobs::save_report(&repo, &forged).unwrap();
        assert!(
            super::super::load_approved_answer(&repo, &work, &report.run)
                .unwrap_err()
                .message
                .contains("immutable result")
        );
        agent_jobs::save_report(&repo, &report).unwrap();
        let mut changed = delivery;
        changed.steps[0].lookup_feedback[0]["status"] = json!("AMBIGUOUS");
        set_state(&mut cp, &changed).unwrap();
        let mut changed_report = report.clone();
        agent_jobs::save_run_checkpoint(&repo, &mut changed_report, &cp).unwrap();
        assert!(super::super::load_approved_answer(&repo, &work, &report.run).is_err());
    }

    #[test]
    fn expanding_reviewer_invalid_exhausted_and_failed_results_are_terminal_without_double_charge()
    {
        if !cfg!(target_os = "macos") {
            return;
        }
        for (mode, calls, status) in [
            ("empty", 3, "DRAFT_REVIEW_INVALID_RESULT"),
            ("unknown", 3, "DRAFT_REVIEW_INVALID_RESULT"),
            ("untracked", 3, "DRAFT_REVIEW_INVALID_RESULT"),
            ("foreign", 3, "DRAFT_REVIEW_FAILED"),
            ("uncertain", 3, "DRAFT_REVIEW_UNCERTAIN"),
            ("overcap", 3, "DRAFT_REVIEW_FAILED"),
            ("approve", 1, "DRAFT_REVIEW_NEEDS_EVIDENCE"),
        ] {
            let (temp, repo, work, author, source) =
                super::super::super::operation_draft::tests::authored_context();
            let path = temp.path().join("review.json");
            let cfg = config(&path, &author, mode, calls);
            let result = run(&repo, &work, &source, &path, None).unwrap();
            assert_eq!(result["status"], status, "{mode}: {result}");
            let report = agent_jobs::latest_report(&repo, &work.id).unwrap().unwrap();
            assert_eq!(report.attempts.len(), 1, "{mode}");
            assert_eq!(checkpoint(&repo, &report, &cfg).phase, "TERMINAL");
            let ledger = json!(account(&repo, &cfg.budget).unwrap());
            assert_eq!(
                run(&repo, &work, &source, &path, None).unwrap(),
                result,
                "{mode}"
            );
            assert_eq!(json!(account(&repo, &cfg.budget).unwrap()), ledger);
            assert!(super::super::load_approved_answer(&repo, &work, &report.run).is_err());
            if calls > 1 {
                assert_eq!(
                    ledger["reservations"]
                        .as_object()
                        .unwrap()
                        .values()
                        .filter(|v| v["status"] == "RELEASED_NOT_DISPATCHED")
                        .count(),
                    2,
                    "{mode}: {ledger}"
                );
            }
            if mode == "approve" {
                let mut forged = checkpoint(&repo, &report, &cfg);
                let mut delivery = state(&forged).unwrap();
                forged.pages.pop();
                delivery.steps[0].after_pages -= 1;
                set_state(&mut forged, &delivery).unwrap();
                let mut changed = report.clone();
                agent_jobs::save_run_checkpoint(&repo, &mut changed, &forged).unwrap();
                assert!(
                    run(&repo, &work, &source, &path, None)
                        .unwrap_err()
                        .message
                        .contains("every native content page")
                );
                assert_eq!(json!(account(&repo, &cfg.budget).unwrap()), ledger);
            }
        }
    }
}
