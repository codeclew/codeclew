//! Optional reader defaults derived from exact accepted author evidence.
//! Canonical fragments and their complete evidence retain their original order.
use super::{
    bindings::Bindings,
    digest,
    model::Operation,
    proposals::{self, Artifact, Claim, Step},
    review::AcceptedVersion,
    store::Repository,
    work::{self, Work},
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// Old publications may have no retained proposal/Work. Missing or mismatched
/// provenance deliberately leaves the reader's legacy selection unchanged.
pub(super) fn project(data: &mut Value, subject: &str, binding: &Bindings, repo: &Repository) {
    let mut contexts = BTreeMap::new();
    let mut preferred = BTreeMap::new();
    if let Some(narrative) = binding.narratives.get(subject) {
        for operation in &narrative.operations {
            let Some(version) = binding
                .accepted_versions
                .get(&format!("{subject}/{}", operation.id))
            else {
                continue;
            };
            let context = contexts.entry(version.proposal.clone()).or_insert_with(|| {
                let artifact = proposals::load(repo, &version.proposal).ok()?;
                let work = work::load(repo, &artifact.work).ok()?;
                Some((artifact, work))
            });
            if let Some((artifact, work)) = context {
                let sources = data["operationSources"]
                    .get(&operation.id)
                    .unwrap_or(&data["sources"]);
                let selected = for_operation(subject, operation, version, artifact, work, sources);
                if !selected.is_empty() {
                    preferred.insert(operation.id.clone(), selected);
                }
            }
        }
    }
    // Recompute rather than trust defaults retained by another reader version.
    data["preferredSourceIds"] = json!(preferred);
}

fn for_operation(
    subject: &str,
    operation: &Operation,
    version: &AcceptedVersion,
    artifact: &Artifact,
    work: &Work,
    sources: &Value,
) -> BTreeMap<String, Vec<String>> {
    let mut selected = BTreeMap::new();
    if artifact.id != version.proposal
        || artifact.work != version.work
        || work.id != version.work
        || work.subject != subject
        || digest(operation).ok().as_ref() != Some(&version.operation_digest)
        || !artifact
            .narrative
            .as_ref()
            .is_some_and(|n| n.subject == subject && n.operations.iter().any(|op| op == operation))
    {
        return selected;
    }
    let mut inputs = artifact.input.operations.iter().filter(|input| {
        proposals::operation_target_id(work, &input.entrypoint, work.handles.get(&input.entrypoint))
            .as_deref()
            == Some(&operation.id)
    });
    let Some(input) = inputs.next() else {
        return selected;
    };
    if inputs.next().is_some() {
        return selected;
    }
    let scope = format!("{subject}/{}", operation.id);
    let records = work.checked.sources();
    let mut add = |slot: &str, claim: &Claim, aliases: Vec<String>| {
        let Ok(id) = proposals::stable(&scope, slot) else {
            return;
        };
        let Some(record) = artifact.claims.get(&id) else {
            return;
        };
        let fragment = &record["fragment"];
        if fragment["id"] != id || fragment["text"] != claim.text {
            return;
        }
        let mut ids = Vec::new();
        for reference in &claim.evidence {
            let Some(handle) = work.handles.get(reference).filter(|h| h.kind == "SOURCE") else {
                continue;
            };
            let Some(record) = records.get(&handle.id) else {
                continue;
            };
            if fragment["sourceIds"]
                .as_array()
                .is_some_and(|values| values.contains(&json!(handle.id)))
                && serde_json::to_value(record).ok().as_ref() == sources.get(&handle.id)
                && !ids.contains(&handle.id)
            {
                ids.push(handle.id.clone());
            }
        }
        if ids.is_empty() {
            return;
        }
        for key in std::iter::once(id).chain(aliases) {
            // Every displayed alias must still be the exact accepted fragment.
            let retained = operation_fragment(operation, &key);
            if retained.is_some_and(|(text, evidence)| {
                text == claim.text && ids.iter().all(|id| evidence.contains(id))
            }) {
                selected.insert(key, ids.clone());
            }
        }
    };
    add("summary", &input.summary, vec![]);
    fn steps(
        scope: &str,
        values: &[Step],
        prefix: &str,
        add: &mut impl FnMut(&str, &Claim, Vec<String>),
    ) {
        for (index, step) in values.iter().enumerate() {
            let slot = format!("{prefix}/{index}");
            add(
                &slot,
                &step.meaning,
                proposals::stable(scope, &format!("paragraph/{slot}"))
                    .into_iter()
                    .collect(),
            );
            steps(scope, &step.children, &format!("{slot}/children"), add);
            steps(scope, &step.otherwise, &format!("{slot}/otherwise"), add);
        }
    }
    steps(&scope, &input.steps, "step", &mut add);
    for (index, claim) in input.explanation.iter().enumerate() {
        if super::sections::contains(&operation.id) {
            add(&format!("section-body/{index}"), claim, vec![]);
        } else {
            add(
                &format!("narrative/{index}"),
                claim,
                proposals::stable(&scope, &format!("paragraph/narrative/{index}"))
                    .into_iter()
                    .collect(),
            );
        }
    }
    for (ci, contract) in input.contracts.iter().enumerate() {
        for (ri, row) in contract.rows.iter().enumerate() {
            add(&format!("contract/{ci}/{ri}"), &row.description, vec![]);
        }
    }
    if let Some(claim) = input
        .assessment
        .as_ref()
        .and_then(|a| a.proposed_correction.as_ref())
    {
        add("correction", claim, vec![]);
    }
    selected
}

fn operation_fragment<'a>(
    operation: &'a Operation,
    id: &str,
) -> Option<(&'a str, &'a Vec<String>)> {
    if operation.summary.id == id {
        return Some((&operation.summary.text, &operation.summary.source_ids));
    }
    for event in &operation.events {
        if event.id == id {
            return Some((&event.text, &event.source_ids));
        }
    }
    for paragraph in &operation.explanation {
        if paragraph.id == id && paragraph.authorship.is_none() {
            return Some((&paragraph.text, &paragraph.source_ids));
        }
    }
    for contract in &operation.interface_contracts {
        for row in &contract.rows {
            if row.id == id {
                return Some((&row.value, &row.source_ids));
            }
        }
    }
    operation
        .assessment
        .as_ref()
        .and_then(|a| a.proposed_correction.as_ref())
        .filter(|f| f.id == id)
        .map(|f| (f.text.as_str(), &f.source_ids))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Materialize the real proposal producer, including dependency-expanded
    // annotation evidence and paragraph aliases, rather than hand-built defaults.
    fn fixture() -> (Work, Artifact, AcceptedVersion, Value) {
        let mut work = work::api_contract_tests::endpoint_context_fixture();
        let entry = work
            .checked
            .services
            .get_mut("orders")
            .unwrap()
            .entrypoints
            .first_mut()
            .unwrap();
        entry.source_ids = vec!["endpoint-source".into(), "service-source".into()];
        work.handles.insert(
            "entry".into(),
            work::Handle {
                kind: "ENTRYPOINT".into(),
                id: entry.id.clone(),
            },
        );
        work.handles.insert(
            "body".into(),
            work::Handle {
                kind: "SOURCE".into(),
                id: "service-source".into(),
            },
        );
        let input = serde_json::from_value(json!({"schema":"codeclew-documentation-proposal/1.0", "operations":[{
            "entrypoint":"entry", "title":"Handle request",
            "summary":{"text":"Call the service", "evidence":["entry", "body"]},
            "steps":[{"kind":"note", "meaning":{"text":"Read the result", "evidence":["entry", "body"]}}],
            "explanation":[{"text":"Return the result", "evidence":["body", "entry"]}]
        }]})).unwrap();
        let state = work::ReadState { work: work.id.clone(), receipts: BTreeMap::from([("all".into(), serde_json::from_value(json!({
            "selection":{}, "resultDigest":"digest", "membershipDigest":"digest", "supplied":["entry","body"], "omitted":[], "nextCursor":null
        })).unwrap())]), ..Default::default() };
        let (narrative, claims, diagnostics) =
            proposals::materialize(&work, &input, &state).unwrap();
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let artifact = Artifact {
            schema: "codeclew-documentation-proposal-artifact/1.0".into(),
            id: "proposal".into(),
            work: work.id.clone(),
            input,
            narrative: Some(narrative),
            status: "READY_WITH_LIMITATIONS".into(),
            diagnostics,
            claims,
            read_digest: "reads".into(),
            influence: work.influence.clone(),
            meaning_review: "UNASSESSED".into(),
        };
        let version = super::super::review::unassessed_versions(&work, &artifact, "reads")
            .unwrap()
            .into_values()
            .next()
            .unwrap();
        let sources = json!(work.checked.sources());
        (work, artifact, version, sources)
    }

    #[test]
    fn exact_accepted_claim_prefers_explicit_body_without_reordering_canonical_evidence() {
        let (work, artifact, version, sources) = fixture();
        let before = serde_json::to_vec(&artifact).unwrap();
        let operation = &artifact.narrative.as_ref().unwrap().operations[0];
        assert_eq!(
            operation.summary.source_ids,
            ["endpoint-source", "service-source"]
        );
        let preferred = for_operation(
            &work.subject,
            operation,
            &version,
            &artifact,
            &work,
            &sources,
        );
        for id in [
            &operation.summary.id,
            &operation.events[0].id,
            &operation.explanation[0].id,
            &operation.explanation[1].id,
        ] {
            assert_eq!(preferred[id], ["service-source"]);
        }
        assert_eq!(serde_json::to_vec(&artifact).unwrap(), before);
    }

    #[test]
    fn wrong_version_operation_claim_or_source_record_never_produces_a_default() {
        let (work, artifact, version, sources) = fixture();
        let operation = &artifact.narrative.as_ref().unwrap().operations[0];
        let check = |artifact: &Artifact, version: &AcceptedVersion, sources: &Value| {
            for_operation(&work.subject, operation, version, artifact, &work, sources)
        };
        let mut wrong = version.clone();
        wrong.proposal = "another".into();
        assert!(check(&artifact, &wrong, &sources).is_empty());
        wrong = version.clone();
        wrong.operation_digest = "another".into();
        assert!(check(&artifact, &wrong, &sources).is_empty());
        wrong = version.clone();
        wrong.work = "another".into();
        assert!(check(&artifact, &wrong, &sources).is_empty());
        let mut changed = sources.clone();
        changed["service-source"]["revision"] = json!("new-revision");
        assert!(check(&artifact, &version, &changed).is_empty());
        let mut legacy = artifact.clone();
        legacy.claims.clear();
        assert!(check(&legacy, &version, &sources).is_empty());
        let mut wrong_claim = artifact.clone();
        wrong_claim.input.operations[0].summary.text = "Different claim".into();
        assert!(!check(&wrong_claim, &version, &sources).contains_key(&operation.summary.id));
        let mut wrong_operation = artifact.clone();
        wrong_operation.input.operations[0].entrypoint = "missing".into();
        assert!(check(&wrong_operation, &version, &sources).is_empty());
    }

    #[test]
    fn missing_legacy_proposal_keeps_all_evidence_and_removes_stale_defaults() {
        let (work, artifact, version, sources) = fixture();
        let narrative = artifact.narrative.as_ref().unwrap();
        let operation = &narrative.operations[0];
        let binding: Bindings = serde_json::from_value(json!({
            "schema":"codeclew-documentation-bindings/1.0", "inputDigest":"input", "renderer":"reader", "extractor":"extractor",
            "influenceScopes":{}, "revisions":{}, "coverage":{}, "catalogues":{}, "fragments":{}, "observations":{},
            "narratives":{&work.subject:narrative}, "outputHashes":{}, "retainedSources":{}, "sectionStates":{},
            "targetRevisions":{}, "updateFailures":{}, "acceptedVersions":{format!("{}/{}",work.subject,operation.id):version}
        })).unwrap();
        let dir = tempfile::tempdir().unwrap();
        Repository::init(dir.path(), "Legacy reader").unwrap();
        let repo = Repository::open(dir.path()).unwrap();
        let mut data = json!({"operations":narrative.operations, "sources":sources, "preferredSourceIds":{"stale":{"claim":["wrong"]}}});
        let canonical = data["operations"].clone();
        project(&mut data, &work.subject, &binding, &repo);
        assert_eq!(data["preferredSourceIds"], json!({}));
        assert_eq!(data["operations"], canonical);
        assert_eq!(data["sources"], sources);
    }
}
