//! Exact request discovery with historical approval and current applicability separated.
use super::{
    agent_jobs::{self, operation_draft_review},
    answer_context, answer_reuse_projection,
    check::Check,
    digest, invalid,
    store::Repository,
    work::{self, Request},
};
use crate::error::ClewError;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;

const SELECTION_SCHEMA: &str = "codeclew-saved-answer-selection/1.0";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Selection {
    schema: String,
    subject: String,
    request_digest: String,
    compared_snapshot: String,
    work: String,
    review_run: String,
}

pub(super) fn find(
    repo: &Repository,
    subject: &str,
    request: Request,
    snapshot: &str,
    selection: Option<Selection>,
) -> Result<Value, ClewError> {
    let current = Check::load_snapshot(repo, snapshot)?;
    let request = answer_reuse_projection::normalized_request(repo, subject, request, &current)?;
    let request_value = serde_json::to_value(&request).map_err(super::io_error)?;
    let request_digest = digest(&request)?;
    let manifests = work::request_manifests(repo)?;
    let work_ids: BTreeSet<String> = manifests.iter().map(|(id, _, _)| id.clone()).collect();
    let approvals = agent_jobs::historical_approved_runs(repo, &work_ids)?;
    let mut candidates = Vec::new();
    let mut answers = Vec::new();
    for (id, saved_subject, saved_request) in manifests {
        if saved_subject != subject
            || serde_json::to_value(&saved_request).map_err(super::io_error)? != request_value
        {
            continue;
        }
        let saved = work::load(repo, &id)?;
        for review_run in approvals.get(&id).into_iter().flatten() {
            if candidates.len() >= 128 {
                return Err(invalid(
                    "answer discovery exceeds 128 historical approvals; use a narrower documentation root",
                ));
            }
            let approved = operation_draft_review::load_approved_answer(repo, &saved, review_run)?;
            let chain = match agent_jobs::validate_reusable_chain(repo, &saved, review_run) {
                Ok(()) => json!({"status":"SUPPORTED"}),
                Err(error) if error.message.starts_with("ANSWER_REUSE_UNSUPPORTED:") => {
                    json!({"status":"UNSUPPORTED","reason":error.message})
                }
                Err(error) => return Err(error),
            };
            let replay = if chain["status"] == "SUPPORTED" {
                match answer_reuse_projection::compare(repo, &saved, &current) {
                    Ok(replay) => replay,
                    Err(error) if error.message.starts_with("ANSWER_REUSE_UNSUPPORTED:") => {
                        json!({"status":"UNKNOWN","reasons":[error.message]})
                    }
                    Err(error) => return Err(error),
                }
            } else {
                json!({"status":"UNKNOWN","reasons":[chain["reason"].clone()]})
            };
            let selected_evidence = answer_context::compare_with_verified_replay(
                &saved,
                &approved.audit,
                &current,
                &replay,
            )?;
            let applicability = combined_status(&selected_evidence, &replay);
            let selector = Selection {
                schema: SELECTION_SCHEMA.into(),
                subject: subject.into(),
                request_digest: request_digest.clone(),
                compared_snapshot: snapshot.into(),
                work: id.clone(),
                review_run: review_run.clone(),
            };
            candidates.push(json!({
                "selection":selector,"savedSnapshot":saved.snapshot,
                "historicalApproval":"MODEL_APPROVED_AGAINST_SAVED_PACKET",
                "applicability":applicability,"selectedEvidence":selected_evidence,
                "initialPreparationReplay":replay,"chain":chain,
                "packetDigest":approved.packet["packetDigest"],
                "answerDigest":digest(&approved.answer)?,
            }));
            answers.push(json!({"selection":selector,"answer":approved.answer,
                "review":approved.review,"provenance":approved.provenance,
                "packet":approved.packet,"audit":approved.audit}));
            if super::bytes(&answers)?.len() > 64 * 1024 * 1024 {
                return Err(invalid(
                    "answer discovery exceeds 64 MiB; use a narrower documentation root",
                ));
            }
        }
    }
    // Enumerators return stable identity order. Sorting never chooses recency or quality.
    let reusable: Vec<usize> = candidates
        .iter()
        .enumerate()
        .filter_map(|(index, candidate)| (candidate["applicability"] == "CURRENT").then_some(index))
        .collect();
    let explicit_index = selection.as_ref().map(|selector| {
        candidates.iter().position(|candidate| candidate["selection"] == json!(selector))
            .ok_or_else(|| invalid("answer selection does not belong to this exact request and compared snapshot; run find-answer again"))
    }).transpose()?;
    let index = match explicit_index {
        Some(index) if reusable.contains(&index) => Some(index),
        Some(_) => {
            return Err(invalid(
                "selected historical answer is not currently reusable; inspect applicability reasons and prepare/review new Work explicitly",
            ));
        }
        None if reusable.len() == 1 => reusable.first().copied(),
        None => None,
    };
    let status = if index.is_some() {
        "FOUND"
    } else if reusable.len() > 1 {
        "SELECTION_REQUIRED"
    } else if candidates.is_empty() {
        "NO_MATCH"
    } else {
        "NO_REUSABLE_MATCH"
    };
    let next_action = match status {
        "FOUND" => {
            "Use selected.answer with its unchanged historical approval and saved citations; current applicability is a separate evidence comparison."
        }
        "SELECTION_REQUIRED" => {
            "Save one candidate.selection object and pass --select <selection.json>; no candidate is preferred automatically."
        }
        "NO_MATCH" => {
            "Prepare Work with this exact request and explicit snapshot, then author and review explicitly."
        }
        _ => {
            "Inspect candidate applicability reasons; acquire a new snapshot separately if needed, then prepare, author and review new Work explicitly."
        }
    };
    Ok(json!({
        "schema":"codeclew-saved-answer-discovery/1.0","status":status,
        "subject":subject,"requestDigest":request_digest,"comparedSnapshot":snapshot,
        "searchComplete":true,"candidates":candidates,
        "selected":index.map(|index| answers[index].clone()),"nextAction":next_action,
        "authority":"HISTORICAL_MODEL_APPROVAL_WITH_EXPLICIT_CAPTURED_CONTEXT_REPLAY",
        "limitations":["CURRENT is applicability to the explicit saved Check plus a read-only protected-notes membership check, not a new meaning review or live runtime verification.",
            "Only method process-graph-v1 authoring 1.6 with source-data context and no role expansions, repair, maintained context or admitted external inputs is supported."],
        "captures":0,"agentInvocations":0,"writes":0,
        "writeScope":"Durable Work, job, review, publication and source records; existing CAS and locking infrastructure may perform operational filesystem IO."
    }))
}

fn combined_status(selected: &Value, replay: &Value) -> &'static str {
    if selected["status"] == "UNKNOWN" || replay["status"] == "UNKNOWN" {
        "UNKNOWN"
    } else if selected["status"] == "STALE" || replay["status"] == "STALE" {
        "STALE"
    } else if selected["status"] == "CURRENT" && replay["status"] == "CURRENT" {
        "CURRENT"
    } else {
        "UNKNOWN"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replay_cannot_be_overridden_by_matching_selected_records() {
        assert_eq!(
            combined_status(&json!({"status":"CURRENT"}), &json!({"status":"STALE"})),
            "STALE"
        );
        assert_eq!(
            combined_status(&json!({"status":"CURRENT"}), &json!({"status":"UNKNOWN"})),
            "UNKNOWN"
        );
        assert_eq!(
            combined_status(&json!({"status":"CURRENT"}), &json!({"status":"CURRENT"})),
            "CURRENT"
        );
    }
}
