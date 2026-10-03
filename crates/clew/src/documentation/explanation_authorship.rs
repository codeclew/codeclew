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
    let (source_refs, dependency_refs, source_snapshot) =
        if let Some(original) = &paragraph.authorship {
            (
                original.source_refs.clone(),
                original.dependency_refs.clone(),
                original.source_snapshot.clone(),
            )
        } else {
            let (sources, dependencies) = refs(paragraph, &work.checked)?;
            (
                sources,
                dependencies,
                work.snapshot.clone().ok_or_else(|| {
                    invalid("authored paragraph requires an immutable source snapshot")
                })?,
            )
        };
    Ok(ExplanationAuthorship {
        authority: ExplanationAuthority::UserDocumentation,
        author: declared.into(),
        meaning_review: AuthoredMeaningReview::Unassessed,
        context_role: AuthoredContextRole::RetainedUnverifiedContext,
        edit_digest: digest(&(&work.id, &work.snapshot, edit))?,
        source_snapshot,
        source_refs,
        dependency_refs,
    })
}
/// Validation checks retained identities, never the meaning of human prose.
pub(super) fn validate_metadata(paragraph: &Explanation) -> Result<(), ClewError> {
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
    Ok(())
}
pub(super) fn validate(paragraph: &Explanation, checked: &Check) -> Result<(), ClewError> {
    validate_metadata(paragraph)?;
    let Some(authorship) = &paragraph.authorship else {
        return Ok(());
    };
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
/// Context identity survives text edits; current generated evidence is separate.
pub(super) fn same_context(old: &Explanation, new: &Explanation) -> bool {
    let (Some(a), Some(b)) = (&old.authorship, &new.authorship) else {
        return false;
    };
    old.id == new.id
        && old.event_ids == new.event_ids
        && old.dependency_ids == new.dependency_ids
        && old.source_ids == new.source_ids
        && old.detail == new.detail
        && a.source_snapshot == b.source_snapshot
        && a.source_refs == b.source_refs
        && a.dependency_refs == b.dependency_refs
}
/// Only the exact canonical authored explanation can own a retained scope.
/// Source IDs stay logical: different versions remain in fragment-owned evidence.
pub(super) fn authored_binding(binding: &Bindings, key: &str) -> Result<bool, ClewError> {
    let Some(fragment) = binding.fragments.get(key) else {
        return Ok(false);
    };
    let Some(paragraph) = binding.narratives.get(&fragment.subject).and_then(|n| {
        n.operations
            .iter()
            .flat_map(|o| o.explanation.iter().map(move |p| (o, p)))
            .find(|(o, p)| key == format!("{}/{}/{}", n.subject, o.id, p.id))
            .map(|(_, p)| p)
    }) else {
        return Ok(false);
    };
    let Some(authorship) = &paragraph.authorship else {
        return Ok(false);
    };
    validate_metadata(paragraph)?;
    if paragraph.source_ids.iter().collect::<BTreeSet<_>>()
        != authorship.source_refs.keys().collect::<BTreeSet<_>>()
    {
        return Err(invalid(
            "authored fragment source IDs do not match pinned context",
        ));
    }
    if fragment.content != serde_json::to_value(paragraph).map_err(super::io_error)?
        || fragment.content_digest != digest(paragraph)?
    {
        return Err(invalid(
            "authored fragment does not match its exact canonical paragraph",
        ));
    }
    let evidence = fragment
        .evidence
        .as_ref()
        .ok_or_else(|| invalid("authored fragment frozen evidence is unavailable"))?;
    let sources: BTreeMap<_, _> = evidence
        .sources
        .iter()
        .map(|(id, s)| Ok((id.clone(), digest(s)?)))
        .collect::<Result<_, ClewError>>()?;
    let dependencies: BTreeMap<_, _> = paragraph
        .dependency_ids
        .iter()
        .map(|id| {
            let observation = evidence
                .observations
                .get(id)
                .ok_or_else(|| invalid("authored fragment original dependency is unavailable"))?;
            Ok((id.clone(), digest(observation)?))
        })
        .collect::<Result<_, ClewError>>()?;
    if sources != authorship.source_refs || dependencies != authorship.dependency_refs {
        return Err(invalid(
            "authored fragment does not match its pinned provenance",
        ));
    }
    Ok(true)
}
pub(super) fn retained_scope(binding: &Bindings, key: &str) -> Result<bool, ClewError> {
    if !authored_binding(binding, key)? {
        return Ok(false);
    }
    let fragment = &binding.fragments[key];
    let scope = fragment
        .influence_scope
        .as_ref()
        .and_then(|id| binding.influence_scopes.get(id))
        .ok_or_else(|| invalid("authored fragment original influence scope is unavailable"))?;
    if fragment
        .dependencies
        .iter()
        .any(|(id, value)| scope.dependencies.get(id) != Some(value))
    {
        return Err(invalid(
            "authored fragment original scope does not cover its frozen dependencies",
        ));
    }
    Ok(true)
}
pub(super) fn validate_binding_pin(
    binding: &Bindings,
    key: &str,
    checked: &Check,
) -> Result<(), ClewError> {
    if !authored_binding(binding, key)? {
        return Err(invalid(
            "only exact canonical authored paragraphs can retain context",
        ));
    }
    let fragment = &binding.fragments[key];
    let paragraph: Explanation =
        serde_json::from_value(fragment.content.clone()).map_err(super::io_error)?;
    validate(&paragraph, checked)?;
    fragment_context(fragment, checked)
}
fn exact_fragment<'a>(
    baseline: &'a Bindings,
    subject: &str,
    operation: &str,
    paragraph: &Explanation,
) -> Result<&'a bindings::FragmentBinding, ClewError> {
    let key = format!("{subject}/{operation}/{}", paragraph.id);
    let fragment = baseline
        .fragments
        .get(&key)
        .ok_or_else(|| invalid("original paragraph binding is unavailable"))?;
    if fragment.content != serde_json::to_value(paragraph).map_err(super::io_error)?
        || fragment.content_digest != digest(paragraph)?
    {
        return Err(invalid("paragraph differs from its exact frozen binding"));
    }
    Ok(fragment)
}
fn fragment_context(
    fragment: &bindings::FragmentBinding,
    checked: &Check,
) -> Result<(), ClewError> {
    if fragment.dependencies.iter().any(|(id, value)| {
        checked
            .dependencies
            .get(id)
            .is_none_or(|o| &o.digest != value)
    }) {
        return Err(invalid(
            "retained explanation linked dependencies changed; explicit context rebase is required",
        ));
    }
    let sources = checked.sources();
    let evidence = fragment
        .evidence
        .as_ref()
        .ok_or_else(|| invalid("retained explanation original frozen evidence is unavailable"))?;
    if evidence
        .sources
        .iter()
        .any(|(id, value)| sources.get(id) != Some(value))
    {
        return Err(invalid(
            "retained explanation linked source bytes changed; explicit context rebase is required",
        ));
    }
    Ok(())
}
pub(super) fn check_edit_context(
    repo: &Repository,
    work: &Work,
    input: &Proposal,
) -> Result<(), ClewError> {
    let (_, baseline) = match bindings::baseline(repo)? {
        Some(baseline) => baseline,
        None if input.retained_edits.is_empty() => return Ok(()),
        None => return Err(invalid("retained edit requires a frozen publication")),
    };
    let mut pinned = BTreeMap::new();
    if let Some(retained) = &work.retained {
        for operation in &retained.operations {
            for paragraph in operation
                .explanation
                .iter()
                .filter(|p| p.authorship.is_some())
            {
                let snapshot = &paragraph.authorship.as_ref().unwrap().source_snapshot;
                if !pinned.contains_key(snapshot) {
                    pinned.insert(snapshot.clone(), Check::load_snapshot(repo, snapshot)?);
                }
                validate(paragraph, &pinned[snapshot])?;
                fragment_context(
                    exact_fragment(&baseline, &work.subject, &operation.id, paragraph)?,
                    &pinned[snapshot],
                )?;
            }
        }
    }
    for edit in input
        .retained_edits
        .iter()
        .filter(|e| e.target == RetainedTarget::ExplanationText)
    {
        let operation = super::work_retained_parts::retained_operation(work, &edit.id)?;
        let paragraph = operation
            .explanation
            .iter()
            .find(|p| Some(p.id.as_str()) == edit.fragment_id.as_deref())
            .ok_or_else(|| invalid("retained explanation fragment is unavailable"))?;
        if paragraph.authorship.is_none() {
            fragment_context(
                exact_fragment(&baseline, &work.subject, &edit.id, paragraph)?,
                &work.checked,
            )?;
        }
    }
    Ok(())
}
/// A normal author supplies generated claims. The host owns exact protected text.
pub(super) fn merge(
    subject: &str,
    old: &Operation,
    generated: &mut Operation,
) -> Result<(), ClewError> {
    for paragraph in old.explanation.iter().filter(|p| p.authorship.is_some()) {
        if paragraph.event_ids.iter().any(|id| {
            !generated
                .events
                .iter()
                .any(|e| &e.id == id && e.kind != "end")
        }) {
            return Err(invalid(
                "generated operation removed a protected paragraph event anchor",
            ));
        }
        if let Some(index) = generated
            .explanation
            .iter()
            .position(|p| p.id == paragraph.id)
        {
            if generated.explanation[index] == *paragraph {
                continue;
            }
            if generated.explanation[index].authorship.is_some() {
                return Err(invalid(
                    "generated operation conflicts with protected paragraph authorship",
                ));
            }
            let id = format!(
                "claim-{}",
                &digest(&(
                    subject,
                    &generated.id,
                    "generated-explanation",
                    &paragraph.id
                ))?[7..27]
            );
            if generated.explanation.iter().any(|p| p.id == id) {
                return Err(invalid(
                    "generated explanation identity collides with protected text",
                ));
            }
            generated.explanation[index].id = id;
        }
        generated.explanation.push(paragraph.clone());
    }
    if generated.explanation.len() > 128 {
        return Err(invalid(
            "preserved and generated explanation exceeds 128 paragraphs",
        ));
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
    let mut pinned = BTreeMap::new();
    for paragraph in operation
        .explanation
        .iter()
        .filter(|p| p.authorship.is_some())
    {
        let snapshot = &paragraph.authorship.as_ref().unwrap().source_snapshot;
        if !pinned.contains_key(snapshot) {
            pinned.insert(snapshot.clone(), Check::load_snapshot(repo, snapshot)?);
        }
        validate(paragraph, &pinned[snapshot])?;
        if let Some(original) = old
            .and_then(|o| o.explanation.iter().find(|p| p.id == paragraph.id))
            .filter(|p| p.authorship.is_some())
        {
            if !same_context(original, paragraph) {
                return Err(invalid(
                    "explanationText cannot silently rebase protected paragraph context",
                ));
            }
            fragment_context(
                exact_fragment(baseline.unwrap(), subject, &operation.id, original)?,
                &pinned[snapshot],
            )?;
        } else {
            validate(paragraph, checked)?;
        }
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
    for id in &changed {
        let original = old
            .unwrap()
            .explanation
            .iter()
            .find(|p| p.id == *id)
            .unwrap();
        if original.authorship.is_none() {
            fragment_context(
                exact_fragment(baseline.unwrap(), subject, &operation.id, original)?,
                checked,
            )?;
        }
    }
    Ok(())
}

/// Derived reader data keeps source versions separate even for the same logical ID.
pub(super) fn project(
    data: &mut serde_json::Value,
    subject: &str,
    binding: &Bindings,
    checked: Option<&Check>,
) {
    let mut sources = BTreeMap::new();
    let mut states = BTreeMap::new();
    if let Some(narrative) = binding.narratives.get(subject) {
        for operation in &narrative.operations {
            for paragraph in operation
                .explanation
                .iter()
                .filter(|p| p.authorship.is_some())
            {
                let key = format!("{subject}/{}/{}", operation.id, paragraph.id);
                let authorship = paragraph.authorship.as_ref().unwrap();
                let evidence = binding
                    .fragments
                    .get(&key)
                    .and_then(|f| f.evidence.as_ref());
                let missing = evidence.is_none()
                    || evidence.is_some_and(|e| {
                        paragraph
                            .source_ids
                            .iter()
                            .any(|id| !e.sources.contains_key(id))
                    });
                let changed = checked
                    .map(|checked| validate(paragraph, checked).is_err())
                    .unwrap_or_else(|| {
                        evidence.is_some_and(|e| {
                            e.revisions.iter().any(|(id, revision)| {
                                binding
                                    .target_revisions
                                    .get(id)
                                    .is_none_or(|target| target.as_ref() != Some(revision))
                            })
                        }) || data["fragmentStates"][&key]["freshness"] == "STALE"
                    });
                let freshness = if missing {
                    "UNVERIFIED"
                } else if changed {
                    "STALE"
                } else {
                    "CURRENT"
                };
                if let Some(evidence) = evidence {
                    sources.insert(key.clone(), serde_json::json!(evidence.sources));
                }
                states.insert(key, serde_json::json!({"freshness":freshness, "verification":"UNASSESSED",
                    "authority":"USER_DOCUMENTATION", "sourceSnapshot":authorship.source_snapshot,
                    "contentRevisions":evidence.map(|e| &e.revisions), "targetRevisions":binding.target_revisions,
                    "reason":if missing { "PINNED_CONTEXT_UNAVAILABLE" } else if changed { "PINNED_CONTEXT_DIFFERS_FROM_SELECTED_SOURCE" } else { "PINNED_CONTEXT_MATCHES_SELECTED_SOURCE" }}));
            }
        }
    }
    if !sources.is_empty() {
        data["fragmentSources"] = serde_json::json!(sources);
    }
    if !states.is_empty() {
        data["fragmentStates"] = serde_json::json!(states);
    }
}
pub(super) fn markdown(data: &serde_json::Value) -> String {
    let mut output = String::new();
    for (key, state) in data["fragmentStates"].as_object().into_iter().flatten() {
        output.push_str(&format!("User documentation paragraph {}: source context freshness {}. Meaning review: UNASSESSED. Pinned source snapshot {}.\n\n",
            super::render::escape(key), state["freshness"].as_str().unwrap_or("UNVERIFIED"),
            super::render::escape(state["sourceSnapshot"].as_str().unwrap_or("unavailable"))));
    }
    output
}
