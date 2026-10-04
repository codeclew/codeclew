//! Explicit catalogue publication of a frozen, model-reviewed operation answer.
//! This is independent of narrative proposals and never dispatches an agent.
use super::{
    agent_jobs,
    bindings::{self, BaselineReceipt, Bindings},
    bytes, digest, history, invalid, io_error, operation_answer, render,
    store::Repository,
    work,
};
use crate::{
    canonical,
    error::{ClewError, ErrorCode},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs};

const MAX_ANSWERS: usize = 64;
const MAX_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "kind",
    rename_all = "SCREAMING_SNAKE_CASE",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ExpectedBaseline {
    None {},
    Exact {
        bundle: String,
        index_digest: String,
        bindings_digest: String,
    },
}
impl ExpectedBaseline {
    fn from_receipt(receipt: Option<&BaselineReceipt>) -> Self {
        receipt.map_or(Self::None {}, |r| Self::Exact {
            bundle: r.bundle.clone(),
            index_digest: r.index_digest.clone(),
            bindings_digest: r.bindings_digest.clone(),
        })
    }
    fn check(&self, receipt: Option<&BaselineReceipt>) -> Result<(), ClewError> {
        if self != &Self::from_receipt(receipt) {
            return Err(conflict(
                "answer publication baseline changed; inspect publication-baseline and select it explicitly",
            ));
        }
        Ok(())
    }
}

/// Owns frozen answer artifacts, separately from narrative fragment sources.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublishedAnswer {
    pub schema: String,
    pub id: String,
    pub title: String,
    pub work: String,
    pub review_run: String,
    pub source_snapshot: String,
    pub packet_digest: String,
    pub answer_digest: String,
    pub review_digest: String,
    pub provenance_digest: String,
    pub meaning_review: String,
    pub publication: String,
    pub source_context: String,
    pub artifact_hashes: BTreeMap<String, String>,
}
impl PublishedAnswer {
    pub fn route(&self) -> String {
        format!("answers/{}.html", self.id)
    }
    fn stem(&self) -> String {
        format!("answers/{}", self.id)
    }
    fn identity(&self) -> Result<String, ClewError> {
        Ok(digest(&(
            self.work.as_str(),
            self.review_run.as_str(),
            self.source_snapshot.as_str(),
            self.packet_digest.as_str(),
            self.answer_digest.as_str(),
            self.review_digest.as_str(),
            self.provenance_digest.as_str(),
        ))?[7..]
            .into())
    }
}
fn conflict(message: &str) -> ClewError {
    ClewError::new(ErrorCode::WwConflict, message)
}
fn hex(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
fn hash(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|s| hex(s, 64))
}

pub(super) fn validate_entries(
    entries: &BTreeMap<String, PublishedAnswer>,
) -> Result<(), ClewError> {
    if entries.len() > MAX_ANSWERS {
        return Err(invalid("reviewed answer catalogue exceeds 64 entries"));
    }
    for (id, entry) in entries {
        if id != &entry.id
            || !hex(id, 64)
            || !hex(&entry.work, 64)
            || !hex(&entry.review_run, 32)
            || entry.schema != "codeclew-reviewed-operation-answer-publication/1.0"
            || entry.title.trim().is_empty()
            || entry.title.len() > 8192
            || entry.meaning_review != "MODEL_APPROVED"
            || entry.publication != "PUBLISHED"
            || entry.source_context != "SAVED_SNAPSHOT_NOT_REVERIFIED"
            || ![
                &entry.packet_digest,
                &entry.answer_digest,
                &entry.review_digest,
                &entry.provenance_digest,
            ]
            .into_iter()
            .all(|s| hash(s))
            || entry.source_snapshot.rsplit_once('/').is_none_or(|(d, n)| {
                !hash(d)
                    || n.parse::<u64>()
                        .ok()
                        .is_none_or(|size| n != size.to_string())
            })
            || entry.identity()? != *id
        {
            return Err(invalid("invalid closed reviewed answer binding"));
        }
        let stem = entry.stem();
        let required = [
            "answer.json",
            "packet.json",
            "audit.json",
            "meaning-review.json",
            "provenance.json",
            "md",
        ];
        if entry.artifact_hashes.len() < required.len()
            || entry.artifact_hashes.len() > required.len() + 1
            || required.iter().any(|suffix| {
                !entry
                    .artifact_hashes
                    .contains_key(&format!("{stem}.{suffix}"))
            })
            || entry.artifact_hashes.iter().any(|(p, h)| {
                !hash(h)
                    || (!required.iter().any(|s| p == &format!("{stem}.{s}"))
                        && p != &format!("{stem}.puml"))
            })
        {
            return Err(invalid(
                "reviewed answer artifact paths or hashes are invalid",
            ));
        }
    }
    Ok(())
}

fn read_bytes(path: &std::path::Path, limit: u64) -> Result<Vec<u8>, ClewError> {
    use std::{io::Read, os::unix::fs::OpenOptionsExt};
    let mut file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(io_error)?;
    let metadata = file.metadata().map_err(io_error)?;
    if !metadata.is_file() || metadata.len() > limit {
        return Err(invalid("publication file is unsafe or exceeds its bound"));
    }
    let mut bytes = Vec::new();
    file.by_ref()
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    if bytes.len() as u64 > limit {
        return Err(invalid("publication file grew beyond its bound"));
    }
    Ok(bytes)
}

pub fn baseline(repo: &Repository) -> Result<Value, ClewError> {
    Ok(json!(ExpectedBaseline::from_receipt(
        bindings::capture_baseline(repo)?.as_ref().map(|(r, _)| r)
    )))
}

/// Carry exact answer sidecars into ordinary render/status bundles too.
pub(super) fn retain_files(
    repo: &Repository,
    previous: Option<&(String, Bindings)>,
    binding: &Bindings,
    files: &mut BTreeMap<String, Vec<u8>>,
) -> Result<(), ClewError> {
    if binding.reviewed_answers.is_empty() {
        return Ok(());
    }
    validate_entries(&binding.reviewed_answers)?;
    let mut owned_bytes = 0u64;
    for entry in binding.reviewed_answers.values() {
        let mut paths = entry.artifact_hashes.clone();
        paths.insert(entry.route(), String::new());
        paths.insert(format!("{}.publication.json", entry.stem()), String::new());
        for (path, expected) in paths {
            if !files.contains_key(&path) {
                let (bundle, prior) =
                    previous.ok_or_else(|| invalid("reviewed answer output is missing"))?;
                if prior.reviewed_answers.get(&entry.id) != Some(entry) {
                    return Err(invalid("retained reviewed answer binding changed"));
                }
                let original = prior.output_hashes.get(&path).ok_or_else(|| {
                    invalid("reviewed answer is missing from retained output manifest")
                })?;
                let bytes = read_bytes(
                    &repo.path(&format!("docs/generated/{bundle}/{path}"))?,
                    MAX_BYTES,
                )?;
                if canonical::hash_bytes(&bytes) != *original {
                    return Err(invalid("retained reviewed answer file was modified"));
                }
                files.insert(path.clone(), bytes);
            }
            owned_bytes = owned_bytes
                .checked_add(files[&path].len() as u64)
                .ok_or_else(|| invalid("answer publication size overflow"))?;
            if !expected.is_empty() && canonical::hash_bytes(&files[&path]) != expected {
                return Err(invalid(
                    "reviewed answer artifact differs from frozen binding",
                ));
            }
        }
    }
    if owned_bytes > MAX_BYTES {
        return Err(invalid(
            "reviewed answer payloads exceed 64 MiB; narrow the retained catalogue",
        ));
    }
    Ok(())
}

fn entry_and_files(
    selected: agent_jobs::operation_draft_review::ReviewedAnswer,
) -> Result<(PublishedAnswer, BTreeMap<String, Vec<u8>>), ClewError> {
    let mut entry = PublishedAnswer {
        schema: "codeclew-reviewed-operation-answer-publication/1.0".into(),
        id: String::new(),
        title: selected.answer["title"].as_str().unwrap_or_default().into(),
        work: selected.provenance["work"]
            .as_str()
            .unwrap_or_default()
            .into(),
        review_run: selected.provenance["reviewRun"]
            .as_str()
            .unwrap_or_default()
            .into(),
        source_snapshot: selected.provenance["snapshot"]
            .as_str()
            .unwrap_or_default()
            .into(),
        packet_digest: digest_without_field(&selected.packet, "packetDigest")?,
        answer_digest: digest(&selected.answer)?,
        review_digest: digest(&selected.review)?,
        provenance_digest: digest(&selected.provenance)?,
        meaning_review: "MODEL_APPROVED".into(),
        publication: "PUBLISHED".into(),
        source_context: "SAVED_SNAPSHOT_NOT_REVERIFIED".into(),
        artifact_hashes: BTreeMap::new(),
    };
    entry.id = entry.identity()?;
    let rendered = operation_answer::validate_and_render_published(
        &selected.packet,
        &selected.audit,
        selected.answer.clone(),
        &selected.provenance,
        &entry.id,
    )?;
    let stem = entry.stem();
    let mut files = BTreeMap::new();
    for (suffix, value) in [
        ("answer.json", &selected.answer),
        ("packet.json", &selected.packet),
        ("audit.json", &selected.audit),
        ("meaning-review.json", &selected.review),
        ("provenance.json", &selected.provenance),
    ] {
        files.insert(format!("{stem}.{suffix}"), bytes(value)?);
    }
    if let Some(diagram) = rendered.process_diagram {
        files.insert(format!("{stem}.puml"), diagram.puml.into_bytes());
    }
    files.insert(format!("{stem}.md"), rendered.markdown.into_bytes());
    entry.artifact_hashes = files
        .iter()
        .map(|(p, b)| (p.clone(), canonical::hash_bytes(b)))
        .collect();
    files.insert(entry.route(), rendered.html.into_bytes());
    files.insert(format!("{stem}.publication.json"), bytes(&entry)?);
    validate_entries(&BTreeMap::from([(entry.id.clone(), entry.clone())]))?;
    Ok((entry, files))
}
fn digest_without_field(value: &Value, field: &str) -> Result<String, ClewError> {
    let mut v = value.clone();
    v.as_object_mut()
        .ok_or_else(|| invalid("invalid frozen JSON"))?
        .remove(field);
    digest(&v)
}

pub(super) fn overview(overview: &str, bundle: &str, binding: &Bindings) -> String {
    if binding.reviewed_answers.is_empty() {
        return overview.to_owned();
    }
    let mut output = overview.to_owned();
    if let Some(start) = output.find("<!-- reviewed-answers-start -->")
        && let Some(end) = output[start..].find("<!-- reviewed-answers-end -->")
    {
        output.replace_range(
            start..start + end + "<!-- reviewed-answers-end -->".len(),
            "",
        );
    }
    let links=binding.reviewed_answers.values().map(|e|format!("<li><a href=\"generated/{}/{}\">{}</a> · MODEL REVIEW: APPROVED · saved context</li>",bundle,e.route(),render::escape(&e.title))).collect::<String>();
    let section = format!(
        "<!-- reviewed-answers-start --><section><h2>Reviewed operation answers</h2><ul>{links}</ul></section><!-- reviewed-answers-end -->"
    );
    output.replacen("</main>", &format!("{section}</main>"), 1)
}

fn retarget_overview_links(overview: &str, old: &str, bundle: &str) -> String {
    overview.replace(
        &format!("href=\"generated/{old}/"),
        &format!("href=\"generated/{bundle}/"),
    )
}

pub fn publish(
    repo: &Repository,
    id: &str,
    review_run: &str,
    expected: ExpectedBaseline,
) -> Result<Value, ClewError> {
    let captured = bindings::capture_baseline(repo)?;
    expected.check(captured.as_ref().map(|(r, _)| r))?;
    let work = work::load(repo, id)?;
    let selected =
        agent_jobs::operation_draft_review::load_approved_answer(repo, &work, review_run)?;
    let (entry, new_files) = entry_and_files(selected)?;
    let input_digest = repo.input_digest()?;
    let previous = captured
        .as_ref()
        .map(|(r, b)| (r.bundle.clone(), b.clone()));
    let receipt = captured.as_ref().map(|(r, _)| r);
    let mut files = BTreeMap::new();
    let mut overview = String::new();
    let previous_bytes = if let Some((r, _)) = &captured {
        let mut budget = history::FrozenInputBudget {
            remaining_bytes: MAX_BYTES,
        };
        let (publication, _) = history::read_frozen_bindings(repo, &r.bundle, &mut budget)?;
        for path in publication.files.keys() {
            if matches!(
                path.as_str(),
                "bindings.json"
                    | "publication.json"
                    | "inputs.json"
                    | "catalog.html"
                    | "overview.html"
                    | "root-overview.html"
            ) {
                continue;
            }
            let copied = read_bytes(
                &repo.path(&format!("docs/generated/{}/{path}", r.bundle))?,
                MAX_BYTES,
            )?;
            if canonical::hash_bytes(&copied) != publication.files[path] {
                return Err(conflict(
                    "frozen publication file changed while being copied",
                ));
            }
            files.insert(path.clone(), copied);
        }
        let raw = read_bytes(&repo.path("docs/index.html")?, 2 * 1024 * 1024)?;
        if canonical::hash_bytes(&raw) != r.index_digest {
            return Err(conflict(
                "catalogue baseline changed while preparing answer",
            ));
        }
        overview = String::from_utf8(raw.clone()).map_err(io_error)?;
        Some(raw)
    } else {
        None
    };
    let mut binding = if let Some((_, b)) = &previous {
        b.clone()
    } else {
        render::make_bindings(&work.checked, BTreeMap::new())?
    };
    if let Some(existing) = binding.reviewed_answers.get(&entry.id) {
        if existing != &entry {
            return Err(invalid(
                "published answer identity differs from exact approved answer",
            ));
        }
        let _lock = repo.lock()?;
        let current = bindings::capture_baseline(repo)?;
        expected.check(current.as_ref().map(|(r, _)| r))?;
        bindings::verify_outputs(repo, &previous.as_ref().unwrap().0, &binding)?;
        return Ok(
            json!({"schema":"codeclew-reviewed-answer-publish/1.0","status":"UNCHANGED","bundle":previous.as_ref().unwrap().0,"answer":entry.id,"route":entry.route(),"baseline":expected,"agentInvocations":0,"captures":0}),
        );
    }
    binding
        .reviewed_answers
        .insert(entry.id.clone(), entry.clone());
    binding.output_hashes.clear();
    files.extend(new_files);
    retain_files(repo, previous.as_ref(), &binding, &mut files)?;
    // The newly selected answer has no prior output; validate after supplying it.
    let bundle = digest(&json!({"binding":binding,"files":files.iter().map(|(p,b)|(p,canonical::hash_bytes(b))).collect::<BTreeMap<_,_>>(),"renderer":render::renderer_digest()?,"released":true}))?[7..].to_owned();
    if let Some((old, _)) = &previous {
        overview = retarget_overview_links(&overview, old, &bundle);
    } else {
        overview = format!(
            "<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{}</title></head><body><main><h1>{}</h1><p>Saved model-reviewed answers; source freshness and runtime execution are not established.</p></main></body></html>",
            render::escape(&repo.manifest.title),
            render::escape(&repo.manifest.title)
        );
    }
    if let Some(start) = overview.find("<!-- reviewed-answers-start -->")
        && let Some(end) = overview[start..].find("<!-- reviewed-answers-end -->")
    {
        overview.replace_range(
            start..start + end + "<!-- reviewed-answers-end -->".len(),
            "",
        );
    }
    let links = binding.reviewed_answers.values().map(|e|format!("<li><a href=\"generated/{}/{}\">{}</a> · MODEL REVIEW: APPROVED · saved context</li>",bundle,e.route(),render::escape(&e.title))).collect::<String>();
    let section = format!(
        "<!-- reviewed-answers-start --><section><h2>Reviewed operation answers</h2><ul>{links}</ul></section><!-- reviewed-answers-end -->"
    );
    overview = overview.replacen("</main>", &format!("{section}</main>"), 1);
    if let Some(first_end) = overview.find('\n')
        && overview.starts_with("<!-- codeclew-bundle ")
    {
        overview.replace_range(..first_end + 1, "");
    }
    overview = format!("<!-- codeclew-bundle {bundle} -->\n{overview}");
    super::progress::run("COMMIT_REVIEWED_ANSWER_PUBLICATION", || {
        render::commit_reviewed_answer_bundle(
            repo,
            &bundle,
            binding,
            files,
            &overview,
            &input_digest,
            previous.as_ref(),
            previous_bytes.as_deref(),
            receipt,
        )
    })?;
    Ok(
        json!({"schema":"codeclew-reviewed-answer-publish/1.0","status":"PUBLISHED","bundle":bundle,"answer":entry.id,"route":format!("docs/generated/{}/{}",bundle,entry.route()),"sourceSnapshot":entry.source_snapshot,"meaningReview":"MODEL_APPROVED","sourceContext":"SAVED_SNAPSHOT_NOT_REVERIFIED","runtime":"UNKNOWN","baseline":baseline(repo)?,"agentInvocations":0,"captures":0}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn overview_retargets_host_links_without_changing_literal_paths_or_external_urls() {
        let overview = concat!(
            "<a href=\"generated/old/services/orders.html\">Service</a>",
            "<p>generated/old/ index.html#anchor process-flow.puml</p>",
            "<a href=\"https://example.invalid/generated/old/index.html#anchor\">External</a>"
        );
        let rewritten = retarget_overview_links(overview, "old", "new");
        assert_eq!(
            rewritten,
            concat!(
                "<a href=\"generated/new/services/orders.html\">Service</a>",
                "<p>generated/old/ index.html#anchor process-flow.puml</p>",
                "<a href=\"https://example.invalid/generated/old/index.html#anchor\">External</a>"
            )
        );
    }

    #[test]
    fn baseline_is_closed_and_empty_answer_binding_preserves_legacy_identity() {
        let none = ExpectedBaseline::None {};
        assert_eq!(
            bytes(&none).unwrap(),
            bytes(&json!({"kind":"NONE"})).unwrap()
        );
        assert_eq!(
            serde_json::from_value::<ExpectedBaseline>(json!({"kind":"NONE"})).unwrap(),
            none
        );
        assert!(
            serde_json::from_value::<ExpectedBaseline>(json!({"kind":"NONE","bundle":"forged"}))
                .is_err()
        );
        assert!(
            serde_json::from_value::<ExpectedBaseline>(json!({"kind":"EXACT","bundle":"a"}))
                .is_err()
        );
        assert!(serde_json::from_value::<ExpectedBaseline>(json!({"kind":"LATEST"})).is_err());
        let work = super::super::work::api_contract_tests::endpoint_context_fixture();
        let mut binding = render::make_bindings(&work.checked, BTreeMap::new()).unwrap();
        bindings::compact(&mut binding);
        let original = bytes(&binding).unwrap();
        assert_eq!(binding.schema, "codeclew-documentation-bindings/1.4");
        let value: Value = serde_json::from_slice(&original).unwrap();
        assert!(value.get("reviewedAnswers").is_none());
        let restored: Bindings = serde_json::from_value(value).unwrap();
        assert_eq!(bytes(&restored).unwrap(), original);
    }
}
