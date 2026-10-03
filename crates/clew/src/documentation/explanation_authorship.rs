//! Declared authorship keeps original linked context without asserting support.
use super::{
    bindings::{self, Bindings},
    check::Check,
    digest, invalid,
    model::*,
    proposals::{self, Proposal, RetainedEdit, RetainedTarget},
    review::AcceptedVersion,
    store::Repository,
    work::Work,
};
use crate::error::ClewError;
use std::collections::{BTreeMap, BTreeSet};

fn author(value: &str) -> Result<(), ClewError> {
    if value.trim().is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
        return Err(invalid(
            "declared author must be nonblank, at most 256 UTF-8 bytes, without control characters",
        ));
    }
    Ok(())
}
type ContextDigests = (BTreeMap<String, String>, BTreeMap<String, String>);

fn refs(paragraph: &Explanation, checked: &Check) -> Result<ContextDigests, ClewError> {
    let sources = checked.sources();
    let source_refs = paragraph
        .source_ids
        .iter()
        .map(|id| {
            let source = sources
                .get(id)
                .ok_or_else(|| invalid("authored paragraph linked source is unavailable"))?;
            Ok((id.clone(), digest(source)?))
        })
        .collect::<Result<_, ClewError>>()?;
    let dependency_refs = paragraph
        .dependency_ids
        .iter()
        .map(|id| {
            let observation = checked
                .dependencies
                .get(id)
                .ok_or_else(|| invalid("authored paragraph linked dependency is unavailable"))?;
            Ok((id.clone(), digest(observation)?))
        })
        .collect::<Result<_, ClewError>>()?;
    Ok((source_refs, dependency_refs))
}
pub(super) fn from_edit(
    work: &Work,
    paragraph: &Explanation,
    edit: &RetainedEdit,
) -> Result<ExplanationAuthorship, ClewError> {
    let declared = edit
        .author
        .as_deref()
        .ok_or_else(|| invalid("explanationText requires a declared author"))?;
    author(declared)?;
    let (source_refs, dependency_refs) = refs(paragraph, &work.checked)?;
    Ok(ExplanationAuthorship {
        authority: ExplanationAuthority::UserDocumentation,
        author: declared.into(),
        meaning_review: AuthoredMeaningReview::Unassessed,
        context_role: AuthoredContextRole::RetainedUnverifiedContext,
        edit_digest: digest(&(&work.id, &work.snapshot, edit))?,
        source_snapshot: work
            .snapshot
            .clone()
            .ok_or_else(|| invalid("authored paragraph requires an immutable source snapshot"))?,
        source_refs,
        dependency_refs,
    })
}
/// Validation checks retained identities, never the meaning of human prose.
pub(super) fn validate(paragraph: &Explanation, checked: &Check) -> Result<(), ClewError> {
    let Some(authorship) = &paragraph.authorship else {
        return Ok(());
    };
    author(&authorship.author)?;
    if authorship.edit_digest.len() != 71
        || !authorship.edit_digest.starts_with("sha256:")
        || !authorship.edit_digest[7..]
            .bytes()
            .all(|b| b.is_ascii_hexdigit())
    {
        return Err(invalid("authored paragraph edit digest is invalid"));
    }
    let (sources, dependencies) = refs(paragraph, checked)?;
    if sources != authorship.source_refs || dependencies != authorship.dependency_refs {
        return Err(invalid(
            "authored paragraph linked context changed; retain its original publication instead of rebinding",
        ));
    }
    Ok(())
}
pub(super) fn preserve(
    old: &Operation,
    new: &Operation,
    allowed: &BTreeSet<&str>,
) -> Result<(), ClewError> {
    for paragraph in old.explanation.iter().filter(|p| p.authorship.is_some()) {
        if !allowed.contains(paragraph.id.as_str())
            && new.explanation.iter().find(|p| p.id == paragraph.id) != Some(paragraph)
        {
            return Err(invalid(
                "operation replacement conflicts with protected user-authored explanation; use an exact retained explanationText edit",
            ));
        }
    }
    Ok(())
}
/// Retained operation cloning must not rebind any of its original linked bytes.
fn bound_context(
    old: &Bindings,
    subject: &str,
    id: &str,
    checked: &Check,
) -> Result<(), ClewError> {
    let prefix = format!("{subject}/{id}/");
    let sources = checked.sources();
    let mut found = false;
    for (_, binding) in old
        .fragments
        .iter()
        .filter(|(key, _)| key.starts_with(&prefix))
    {
        found = true;
        if binding.dependencies.iter().any(|(id, value)| {
            checked
                .dependencies
                .get(id)
                .is_none_or(|o| &o.digest != value)
        }) {
            return Err(invalid(
                "retained explanation linked dependencies changed; prepare against its original linked context",
            ));
        }
        let evidence = binding
            .evidence
            .as_ref()
            .ok_or_else(|| invalid("retained explanation has no exact frozen source context"))?;
        if evidence
            .sources
            .iter()
            .any(|(id, value)| sources.get(id) != Some(value))
        {
            return Err(invalid(
                "retained explanation linked source bytes changed; prepare against its original linked context",
            ));
        }
    }
    if !found {
        return Err(invalid(
            "retained explanation original binding context is unavailable",
        ));
    }
    Ok(())
}
pub(super) fn check_edit_context(
    repo: &Repository,
    work: &Work,
    input: &Proposal,
) -> Result<(), ClewError> {
    let ids: BTreeSet<_> = input
        .retained_edits
        .iter()
        .filter(|e| e.target == RetainedTarget::ExplanationText)
        .map(|e| e.id.as_str())
        .collect();
    if ids.is_empty() {
        return Ok(());
    }
    let (_, binding) = bindings::baseline(repo)?
        .ok_or_else(|| invalid("retained explanation requires a frozen publication"))?;
    for id in ids {
        bound_context(&binding, &work.subject, id, &work.checked)?;
    }
    Ok(())
}
/// Every changed authored paragraph must come from the existing manual proposal
/// artifact and UNASSESSED version; direct narrative input cannot invent authority.
pub(super) fn admit(
    repo: &Repository,
    baseline: Option<&Bindings>,
    subject: &str,
    operation: &Operation,
    checked: &Check,
    version: Option<&AcceptedVersion>,
) -> Result<(), ClewError> {
    let old = baseline
        .and_then(|b| b.narratives.get(subject))
        .and_then(|n| n.operations.iter().find(|o| o.id == operation.id));
    let changed: BTreeSet<_> = operation
        .explanation
        .iter()
        .filter(|p| {
            p.authorship.is_some()
                && old.and_then(|o| o.explanation.iter().find(|old| old.id == p.id)) != Some(*p)
        })
        .map(|p| p.id.as_str())
        .collect();
    if let Some(old) = old {
        preserve(old, operation, &changed)?;
    }
    for paragraph in operation
        .explanation
        .iter()
        .filter(|p| p.authorship.is_some())
    {
        validate(paragraph, checked)?;
        // The existing cached Check owns the original bytes; metadata is not a new store.
        let snapshot = Check::load_snapshot(
            repo,
            &paragraph.authorship.as_ref().unwrap().source_snapshot,
        )?;
        validate(paragraph, &snapshot)?;
    }
    if changed.is_empty() {
        return Ok(());
    }
    let version = version
        .filter(|v| v.verification == "UNASSESSED")
        .ok_or_else(|| {
            invalid("changed user documentation requires a bound manual UNASSESSED proposal")
        })?;
    let proposal = proposals::load(repo, &version.proposal)?;
    if proposal.work != version.work
        || proposal.meaning_review != "UNASSESSED"
        || proposal
            .narrative
            .as_ref()
            .filter(|n| n.subject == subject)
            .and_then(|n| n.operations.iter().find(|o| o.id == operation.id))
            != Some(operation)
    {
        return Err(invalid(
            "user documentation does not match its bound manual proposal",
        ));
    }
    let work = super::work::load(repo, &proposal.work)?;
    for id in &changed {
        let edit = proposal
            .input
            .retained_edits
            .iter()
            .find(|e| {
                e.id == operation.id
                    && e.target == RetainedTarget::ExplanationText
                    && e.fragment_id.as_deref() == Some(*id)
            })
            .ok_or_else(|| {
                invalid("user documentation requires an exact retained explanationText instruction")
            })?;
        let original = old
            .and_then(|o| o.explanation.iter().find(|p| p.id == *id))
            .ok_or_else(|| invalid("retained explanation original paragraph is unavailable"))?;
        let paragraph = operation.explanation.iter().find(|p| p.id == *id).unwrap();
        if original.text != edit.expected_old_value
            || paragraph.text != edit.replacement
            || paragraph.authorship.as_ref() != Some(&from_edit(&work, original, edit)?)
        {
            return Err(invalid(
                "authored paragraph instruction or provenance does not match its exact baseline",
            ));
        }
        let mut expected = original.clone();
        expected.text = paragraph.text.clone();
        expected.authorship = paragraph.authorship.clone();
        if &expected != paragraph {
            return Err(invalid(
                "retained paragraph edit changed fields outside text and declared authorship",
            ));
        }
    }
    bound_context(
        baseline.ok_or_else(|| invalid("retained explanation baseline unavailable"))?,
        subject,
        &operation.id,
        checked,
    )
}
