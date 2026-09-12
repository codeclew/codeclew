//! A status publication retains content evidence; it does not silently rebind prose.
use super::{
    bindings::{self, Bindings},
    bytes,
    check::{self, Check},
    digest, invalid, io_error,
    model::*,
    render,
    store::{self, Repository},
};
use crate::error::ClewError;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
};

pub fn update_states(binding: &mut Bindings, checked: &Check) {
    let report = bindings::freshness(Some(binding), checked);
    let mut keys: BTreeSet<String> = binding.narratives.keys().cloned().collect();
    for (subject, narrative) in &binding.narratives {
        keys.extend(
            narrative
                .operations
                .iter()
                .map(|o| format!("{subject}/{}", o.id)),
        );
    }
    let targets: BTreeMap<_, _> = binding
        .revisions
        .keys()
        .chain(checked.services.keys())
        .chain(checked.unresolved.keys())
        .map(|id| {
            (
                id.clone(),
                checked
                    .services
                    .get(id)
                    .map(|e| e.revision.clone())
                    .or_else(|| {
                        checked
                            .unresolved
                            .get(id)
                            .and_then(|v| v["targetRevision"].as_str())
                            .map(str::to_owned)
                    }),
            )
        })
        .collect();
    let mut states = BTreeMap::new();
    for key in keys {
        let subject = key.split('/').next().unwrap_or(&key);
        let prefix = format!("{key}/");
        let mut services = BTreeSet::new();
        let mut content_versions: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        if let Some(id) = subject.strip_prefix("service:") {
            services.insert(id.to_owned());
        }
        for (id, fragment) in &binding.fragments {
            if id.starts_with(&prefix) {
                if let Some(evidence) = &fragment.evidence {
                    for (id, revision) in &evidence.revisions {
                        content_versions
                            .entry(id.clone())
                            .or_default()
                            .insert(revision.clone());
                    }
                }
                let observations = fragment
                    .evidence
                    .as_ref()
                    .map(|e| &e.observations)
                    .unwrap_or(&binding.observations);
                for dep in fragment.dependencies.keys() {
                    if let Some(observation) = observations.get(dep) {
                        if !observation.service.is_empty() {
                            services.insert(observation.service.clone());
                        }
                        if observation.kind == "DECLARED_INTERACTION" {
                            for side in ["from", "to"] {
                                if let Some(s) = observation.normalized[side]["service"].as_str() {
                                    services.insert(s.into());
                                }
                            }
                        }
                    }
                }
                for source in fragment
                    .sources
                    .keys()
                    .filter_map(|id| binding.retained_sources.get(id))
                {
                    services.insert(source.service.clone());
                }
            }
        }
        if services.is_empty() {
            services.extend(binding.revisions.keys().cloned());
        }
        let mut reasons: Vec<Value> = report["affected"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|f| {
                f["fragment"]
                    .as_str()
                    .is_some_and(|id| id.starts_with(&prefix))
            })
            .flat_map(|f| f["reasons"].as_array().cloned().unwrap_or_default())
            .collect();
        reasons.extend(
            report["catalogueChanges"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|c| c["service"].as_str().is_some_and(|s| services.contains(s)))
                .cloned(),
        );
        let unavailable = services.iter().any(|id| {
            checked.unresolved.contains_key(id)
                || checked
                    .services
                    .get(id)
                    .is_some_and(|e| e.extractor == SOURCE_EXTRACTOR && e.coverage != "SYNTAX")
        });
        for id in &services {
            if let Some(failure) = checked.unresolved.get(id) {
                reasons.push(json!({"reason":"SOURCE_UNAVAILABLE","service":id,"details":failure}));
            } else if !checked.services.contains_key(id) {
                reasons.push(json!({"reason":"SERVICE_REMOVED","service":id}));
            }
        }
        let previous = binding.section_states.get(&key);
        // A status operation cannot clear a prior review obligation merely by losing its inputs.
        if reasons.is_empty() && previous.is_some_and(|s| s.freshness != Freshness::Current) {
            reasons.extend(previous.unwrap().reasons.clone());
            if reasons.is_empty() {
                reasons.push(json!({"reason":"PRIOR_REVIEW_REQUIRED"}));
            }
        }
        reasons.sort_by_key(Value::to_string);
        reasons.dedup();
        let freshness = if unavailable {
            Freshness::Unverified
        } else if reasons.is_empty() {
            Freshness::Current
        } else {
            Freshness::Stale
        };
        states.insert(
            key,
            SectionState {
                freshness,
                verification: previous
                    .map(|s| s.verification.clone())
                    .unwrap_or_else(|| "UNASSESSED".into()),
                content_revisions: services
                    .iter()
                    .filter_map(|id| match content_versions.get(id) {
                        Some(values) if values.len() == 1 => {
                            Some((id.clone(), values.first().unwrap().clone()))
                        }
                        Some(_) => None,
                        None => previous
                            .and_then(|s| s.content_revisions.get(id))
                            .or_else(|| binding.revisions.get(id))
                            .map(|r| (id.clone(), r.clone())),
                    })
                    .collect(),
                mixed_revisions: content_versions
                    .iter()
                    .filter(|(_, values)| values.len() > 1)
                    .map(|(id, values)| (id.clone(), values.iter().cloned().collect()))
                    .collect(),
                target_revisions: services
                    .iter()
                    .map(|id| (id.clone(), targets.get(id).cloned().flatten()))
                    .collect(),
                coverage: services
                    .iter()
                    .filter_map(|id| binding.coverage.get(id).map(|v| (id.clone(), v.clone())))
                    .collect(),
                reasons,
            },
        );
    }
    binding.section_states = states;
    binding.target_revisions = targets;
}

pub(super) fn attach(data: &mut Value, subject: &str, binding: &Bindings) {
    data["sectionState"] = json!(binding.section_states.get(subject));
    data["operationStates"] = json!(
        binding
            .section_states
            .iter()
            .filter_map(|(id, s)| id.strip_prefix(&format!("{subject}/")).map(|op| (op, s)))
            .collect::<BTreeMap<_, _>>()
    );
    data["targetRevisions"] = json!(binding.target_revisions);
}

pub fn refresh(repo: &Repository) -> Result<Value, ClewError> {
    let previous = bindings::baseline(repo)?
        .ok_or_else(|| invalid("no retained publication; run docs render first"))?;
    bindings::verify_outputs(repo, &previous.0, &previous.1)?;
    let previous_bytes = fs::read(repo.path("docs/index.html")?).map_err(io_error)?;
    let checked = check::run(repo)?;
    checked.save(repo)?;
    let mut binding = previous.1.clone();
    update_states(&mut binding, &checked);
    binding.schema = "codeclew-documentation-bindings/1.1".into();
    binding.output_hashes.clear();
    let bundle = digest(&json!({"binding":binding,"targetInput":checked.input_digest,"statusRenderer":render::renderer_digest()?}))?[7..].to_owned();
    let mut files = BTreeMap::new();
    let mut cards = String::new();
    for subject in binding.narratives.keys() {
        let (kind, id) = subject
            .split_once(':')
            .ok_or_else(|| invalid("invalid retained subject"))?;
        let folder = if kind == "service" {
            "services"
        } else {
            "scenarios"
        };
        let path = format!("{folder}/{id}.json");
        let mut data: Value = store::read(
            &repo.path(&format!("docs/generated/{}/{path}", previous.0))?,
            64 * 1024 * 1024,
        )?;
        attach(&mut data, subject, &binding);
        super::notes::mark_targets(&mut data, &checked);
        super::processes::mark_targets(&mut data, &checked, subject);
        super::dataflow::mark_targets(&mut data, &checked, subject);
        files.insert(path, bytes(&data)?);
        files.insert(
            format!("{folder}/{id}.html"),
            render::html(&data)?.into_bytes(),
        );
        let state = &binding.section_states[subject];
        let state_label = match state.freshness {
            Freshness::Current => "CURRENT",
            Freshness::Stale => "STALE",
            Freshness::Unverified => "UNVERIFIED",
        };
        let title = data["title"].as_str().unwrap_or(id);
        let body = render::markdown(title, &binding.narratives[subject], &binding.section_states)
            + &super::notes::markdown(&data["notes"])
            + &super::processes::markdown(&data["process"])
            + &super::dataflow::markdown(&data["view"], &binding.narratives[subject]);
        let status_text = format!(
            "Source freshness: {state_label}. Meaning review: {}.\n\nContent revisions: {}\n\nTarget revisions: {}\n\n",
            state.verification,
            serde_json::to_string(&state.content_revisions).map_err(io_error)?,
            serde_json::to_string(&state.target_revisions).map_err(io_error)?
        );
        files.insert(
            format!("{folder}/{id}.md"),
            format!("{status_text}{body}").into_bytes(),
        );
        cards.push_str(&format!("<article class=\"gap-card\"><h2><a href=\"generated/{bundle}/{folder}/{}.html\">{}</a></h2><p class=\"freshness-status\">{state_label}</p><p>Retained explanation; source status checked independently.</p><details><summary>Revisions and missing information</summary><pre>{}</pre></details></article>",render::escape(id),render::escape(data["title"].as_str().unwrap_or(id)),render::escape(&serde_json::to_string_pretty(state).map_err(io_error)?)));
    }
    for (subject, narrative) in &binding.narratives {
        for operation in &narrative.operations {
            let state = &binding.section_states[&format!("{subject}/{}", operation.id)];
            let label = match state.freshness {
                Freshness::Current => "CURRENT",
                Freshness::Stale => "STALE",
                Freshness::Unverified => "UNVERIFIED",
            };
            files.insert(
                format!(
                    "diagrams/{}-{}.mmd",
                    subject.replace(':', "-"),
                    operation.id
                ),
                format!(
                    "%% Source freshness: {label}; meaning review: {}\n{}",
                    state.verification,
                    render::mermaid(operation)
                )
                .into_bytes(),
            );
        }
    }
    files.insert("status.json".into(), bytes(&json!({"schema":"codeclew-documentation-status/1.0","sections":binding.section_states,"targetRevisions":binding.target_revisions}))?);
    let overview = format!(
        "<!-- codeclew-bundle {bundle} -->\n<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{}</title><style>{}</style></head><body><main style=\"margin:auto;max-width:1180px;padding:32px\"><h1>{}</h1><p>Source freshness updated. Retained prose has not been regenerated or newly verified.</p>{cards}</main></body></html>\n",
        render::escape(&repo.manifest.title),
        render::STYLE,
        render::escape(&repo.manifest.title)
    );
    let sections = binding.section_states.clone();
    render::commit_bundle(
        repo,
        &bundle,
        binding,
        files,
        &overview,
        &checked.input_digest,
        Some(&previous),
        Some(&previous_bytes),
    )?;
    Ok(
        json!({"schema":"codeclew-docs-refresh/1.0","status":"PUBLISHED","statusOnly":true,"bundle":bundle,"index":"docs/index.html","sections":sections,"unresolved":checked.unresolved,"agentInvocations":0}),
    )
}
