//! Kotlin qualification adapter for the shared allocation/binding proof.
//! It consumes the same closed original-byte input as data-state projection.
#[cfg(not(test))]
use super::super::model::Gap;
#[cfg(test)]
use super::super::model::Gap;
use super::{Context, gap};
use crate::documentation::{digest, model::Observation};

pub(in crate::documentation) fn retained_storage_source(
    context: &Context<'_>,
    observation: &Observation,
) -> Result<crate::documentation::model::Source, Gap> {
    let site = &observation.normalized["outlineOwnerSource"];
    let source = site["sourceId"]
        .as_str()
        .and_then(|id| context.evidence.sources.get(id))
        .filter(|s| {
            context.valid_source(s)
                && s.service == context.evidence.service
                && s.revision == context.evidence.revision
        })
        .ok_or_else(|| {
            gap(
                "CONSTRUCTOR_SOURCE_UNAVAILABLE",
                "Exact original storage/constructor source is unavailable.",
                None,
            )
        })?;
    if observation.digest
        != digest(&observation.normalized).map_err(|_| {
            gap(
                "CONSTRUCTOR_SOURCE_UNAVAILABLE",
                "Storage digest cannot be validated.",
                None,
            )
        })?
        || observation.source_ids != [source.id.clone()]
        || site["sourceStatus"] != "SOURCE_RETAINED"
        || site["file"] != source.file
        || site["sourceDigest"] != source.text_digest
        || site["evidenceDigest"] != source.evidence_digest
        || site["byteEnd"]
            .as_u64()
            .zip(site["byteStart"].as_u64())
            .and_then(|(end, start)| end.checked_sub(start))
            != Some(source.text.len() as u64)
    {
        return Err(gap(
            "CONSTRUCTOR_SOURCE_UNAVAILABLE",
            "Storage/constructor original source pins disagree.",
            None,
        ));
    }
    Ok(source.clone())
}
