//! Declared authorship keeps original linked context without asserting support.
use super::{
    bindings::{self, Bindings},
    check::Check,
    digest, invalid,
    model::*,
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

/// Digest only the effective context, independently of text attribution.
pub(super) fn context_digest(paragraph: &Explanation) -> Result<String, ClewError> {
    let a = paragraph
        .authorship
        .as_ref()
        .ok_or_else(|| invalid("context migration requires an authored paragraph"))?;
    digest(&(
        &a.source_snapshot,
        &a.source_refs,
        &a.dependency_refs,
        &a.context_role,
        &a.context_migration,
    ))
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
    if let Some(migration) = &authorship.context_migration {
        author(&migration.editor)?;
        for value in [
            &migration.instruction_digest,
            &migration.previous_context_digest,
        ] {
            if value.len() != 71
                || !value.starts_with("sha256:")
                || !value[7..].bytes().all(|b| b.is_ascii_hexdigit())
            {
                return Err(invalid("context migration digest is invalid"));
            }
        }
        let valid_snapshot = migration
            .previous_source_snapshot
            .rsplit_once('/')
            .is_some_and(|(hash, size)| {
                hash.len() == 71
                    && hash.starts_with("sha256:")
                    && hash[7..].bytes().all(|b| b.is_ascii_hexdigit())
                    && size.parse::<u64>().is_ok_and(|value| {
                        value > 0
                            && value <= super::check::PORTABLE_CACHE_MAX_BYTES
                            && size == value.to_string()
                    })
            });
        if !valid_snapshot {
            return Err(invalid("context migration previous snapshot is invalid"));
        }
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
        && a.context_migration == b.context_migration
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
