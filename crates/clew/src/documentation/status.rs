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
use crate::error::{ClewError, ErrorCode};
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
                if let Some(scope) = fragment
                    .influence_scope
                    .as_ref()
                    .and_then(|id| binding.influence_scopes.get(id))
                {
                    for observation in scope.declarations.values() {
                        if !observation.service.is_empty() {
                            services.insert(observation.service.clone());
                        }
                        if observation.kind == "DECLARED_INTERACTION" {
                            for side in ["from", "to"] {
                                if let Some(service) =
                                    observation.normalized[side]["service"].as_str()
                                {
                                    services.insert(service.into());
                                }
                            }
                        }
                    }
                    if let Some(evidence) = &fragment.evidence {
                        services.extend(evidence.revisions.keys().cloned());
                    }
                }
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
        let has_new_reasons = !reasons.is_empty();
        let retained_stale = previous.is_some_and(|s| s.freshness == Freshness::Stale);
        // Carrying an unknown state is not new evidence of a change. Conversely,
        // losing inputs must never erase an already observed stale obligation.
        if (retained_stale || !has_new_reasons)
            && previous.is_some_and(|s| s.freshness != Freshness::Current)
        {
            reasons.extend(previous.unwrap().reasons.clone());
            if reasons.is_empty() {
                reasons.push(json!({"reason":"PRIOR_REVIEW_REQUIRED"}));
            }
        }
        reasons.sort_by_key(Value::to_string);
        reasons.dedup();
        let freshness = if retained_stale {
            Freshness::Stale
        } else if unavailable {
            Freshness::Unverified
        } else if !has_new_reasons {
            previous
                .map(|s| s.freshness.clone())
                .unwrap_or(Freshness::Current)
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

fn observation_reason(mut reason: Value) -> Value {
    reason["authority"] = json!("TARGET_OBSERVATION");
    reason
}

/// Compare directly authored records only; never rebuild compiler or process graphs.
fn changed_declarations(
    repo: &Repository,
    binding: &Bindings,
) -> Result<BTreeSet<String>, ClewError> {
    let notes = super::notes::snapshot(repo)?;
    let scenarios = repo.scenarios()?;
    let interactions = repo.interactions()?;
    let mut changed = BTreeSet::new();
    for observation in binding
        .influence_scopes
        .values()
        .flat_map(|scope| scope.declarations.values())
        .chain(binding.observations.values())
        .chain(
            binding
                .fragments
                .values()
                .filter_map(|fragment| fragment.evidence.as_ref())
                .flat_map(|evidence| evidence.observations.values()),
        )
    {
        let value = &observation.normalized;
        let differs = match observation.kind.as_str() {
            "NOTE_ASSOCIATION" => notes
                .get(observation.id.strip_prefix("note:").unwrap_or(""))
                .is_none_or(|note| {
                    note["associationDigest"] != value["associationDigest"]
                        || note["original"] != value["original"]
                }),
            "DECLARED_INTERACTION" => {
                interactions
                    .get(observation.id.strip_prefix("interaction:").unwrap_or(""))
                    .map(serde_json::to_value)
                    .transpose()
                    .map_err(io_error)?
                    .as_ref()
                    != Some(value)
            }
            "SCENARIO_SELECTION" => {
                scenarios
                    .get(observation.id.strip_prefix("scenario:").unwrap_or(""))
                    .map(serde_json::to_value)
                    .transpose()
                    .map_err(io_error)?
                    .as_ref()
                    != Some(value)
            }
            "PROCESS_DEFINITION" | "VIEW_DEFINITION" => {
                scenarios
                    .get(value["definition"]["id"].as_str().unwrap_or(""))
                    .map(serde_json::to_value)
                    .transpose()
                    .map_err(io_error)?
                    .as_ref()
                    != Some(&value["definition"])
            }
            "PROCESS_SCOPE" => {
                value["interactionMembership"] != json!(interactions.keys().collect::<Vec<_>>())
            }
            "VIEW_SCOPE" => {
                let id = observation.id.strip_prefix("view-scope:").unwrap_or("");
                scenarios
                    .get(id)
                    .and_then(|scenario| scenario.view.as_ref())
                    .is_none_or(|view| {
                        value["interactionMembership"]
                            != json!(
                                interactions
                                    .values()
                                    .filter(|link| view.services.contains(&link.from.service)
                                        && view.services.contains(&link.to.service))
                                    .map(|link| &link.id)
                                    .collect::<Vec<_>>()
                            )
                    })
            }
            _ => false,
        };
        if differs {
            changed.insert(observation.id.clone());
        }
    }
    Ok(changed)
}

fn recorded_input_changes(
    repo: &Repository,
    binding: &Bindings,
) -> Result<BTreeMap<String, Vec<Value>>, ClewError> {
    let changed = changed_declarations(repo, binding)?;
    let mut affected: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for key in binding.section_states.keys() {
        let prefix = format!("{key}/");
        let dependencies: BTreeSet<_> = binding
            .fragments
            .iter()
            .filter(|(id, _)| id.starts_with(&prefix))
            .flat_map(|(_, fragment)| {
                fragment
                    .dependencies
                    .keys()
                    .chain(
                        fragment
                            .influence_scope
                            .as_ref()
                            .and_then(|id| binding.influence_scopes.get(id))
                            .into_iter()
                            .flat_map(|scope| scope.dependencies.keys()),
                    )
                    .cloned()
            })
            .collect();
        for id in dependencies.intersection(&changed) {
            affected.entry(key.clone()).or_default().push(observation_reason(json!({"reason":"RECORDED_DECLARATION_CHANGED","dependency":id,"requiresRecheck":true})));
        }
    }
    // Many operations share the same authoring inputs. Read each selection
    // once per observation, without carrying cached fingerprints to a later run.
    let mut fingerprints = BTreeMap::new();
    for (key, version) in &binding.accepted_versions {
        let selection: BTreeSet<_> = version.external_request.external_inputs.iter().collect();
        let fingerprint = match fingerprints.entry(selection) {
            std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::btree_map::Entry::Vacant(entry) => entry.insert(digest(
                &super::work::capture_inputs(repo, &version.external_request)?,
            )?),
        };
        if *fingerprint != version.external_fingerprint {
            let reason = observation_reason(
                json!({"reason":"REGISTERED_INPUTS_CHANGED","requiresRecheck":true}),
            );
            affected
                .entry(key.clone())
                .or_default()
                .push(reason.clone());
            if let Some((subject, _)) = key.split_once('/') {
                affected.entry(subject.into()).or_default().push(reason);
            }
        }
    }
    Ok(affected)
}

/// Observe only known publication participants and declared target refs. This
/// never captures evidence or treats matching revisions as semantic freshness.
/// Content evidence and independent meaning-review authority remain unchanged.
pub fn observe(repo: &Repository, binding: &mut Bindings) -> Result<Value, ClewError> {
    let input_digest = repo.input_digest()?;
    let services = repo.services()?;
    let selected = super::updates::state(repo)?;
    let policies = super::updates::policies(repo)?;
    let participants: BTreeSet<String> = binding
        .revisions
        .keys()
        .cloned()
        .chain(binding.section_states.values().flat_map(|s| {
            s.content_revisions
                .keys()
                .cloned()
                .chain(s.mixed_revisions.keys().cloned())
                .chain(
                    s.target_revisions
                        .iter()
                        .filter_map(|(id, revision)| revision.as_ref().map(|_| id.clone())),
                )
        }))
        .collect();
    let mut targets = BTreeMap::new();
    let mut reasons = BTreeMap::new();
    for id in &participants {
        let (revision, reason) = if let Some(service) = services.get(id) {
            if let Some(target) = selected.targets.get(id) {
                if target.repository_id == service.repository_id
                    && policies.get(id).is_some_and(|policy| {
                        policy.repository_id == service.repository_id
                            && policy.accepted_refs.contains(&target.source_ref)
                    })
                {
                    (
                        Some(target.revision.clone()),
                        json!({"reason":"SELECTED_TARGET_NOT_RECHECKED","service":id,"sourceRef":target.source_ref}),
                    )
                } else {
                    (
                        None,
                        json!({"reason":"SELECTED_TARGET_NOT_ADMITTED","service":id}),
                    )
                }
            } else {
                match super::analysis::bound_repository(repo, service).and_then(|source| {
                    super::analysis::git(
                        &source,
                        &[
                            "rev-parse",
                            "--verify",
                            &format!("{}^{{commit}}", service.target_ref),
                        ],
                    )
                }) {
                    Ok(revision) => (
                        Some(revision),
                        json!({"reason":"LOCAL_TARGET_REF_NOT_RECHECKED","service":id,"sourceRef":service.target_ref}),
                    ),
                    Err(error) => (
                        None,
                        json!({"reason":"LOCAL_TARGET_UNAVAILABLE","service":id,"sourceRef":service.target_ref,"code":error.code}),
                    ),
                }
            }
        } else {
            (None, json!({"reason":"SERVICE_REMOVED","service":id}))
        };
        targets.insert(id.clone(), revision);
        reasons.insert(id.clone(), observation_reason(reason));
    }
    let keys: BTreeSet<String> = binding
        .narratives
        .iter()
        .flat_map(|(subject, narrative)| {
            std::iter::once(subject.clone()).chain(
                narrative
                    .operations
                    .iter()
                    .map(move |op| format!("{subject}/{}", op.id)),
            )
        })
        .collect();
    for key in keys {
        binding
            .section_states
            .entry(key)
            .or_insert_with(|| SectionState {
                freshness: Freshness::Unverified,
                verification: "UNASSESSED".into(),
                content_revisions: binding.revisions.clone(),
                mixed_revisions: BTreeMap::new(),
                target_revisions: BTreeMap::new(),
                coverage: binding.coverage.clone(),
                reasons: Vec::new(),
            });
    }
    let declarations_changed = input_digest != binding.input_digest;
    let recorded_changes = recorded_input_changes(repo, binding)?;
    for (key, state) in &mut binding.section_states {
        let ids: BTreeSet<String> = state
            .content_revisions
            .keys()
            .cloned()
            .chain(state.mixed_revisions.keys().cloned())
            .chain(state.target_revisions.keys().cloned())
            .collect();
        // Replace transient observation reasons; preserve independent semantic
        // review obligations and the sticky stale state until an explicit recheck.
        state
            .reasons
            .retain(|reason| reason["authority"] != "TARGET_OBSERVATION");
        let mut changed = state.freshness == Freshness::Stale;
        if let Some(reasons) = recorded_changes.get(key) {
            changed = true;
            state.reasons.extend(reasons.clone());
        }
        if declarations_changed {
            state.reasons.push(observation_reason(
                json!({"reason":"DECLARATION_SCOPE_NOT_RECHECKED","requiresRecheck":true}),
            ));
        }
        for id in &ids {
            if let Some(reason) = reasons.get(id) {
                state.reasons.push(reason.clone());
            }
            if !services.contains_key(id) {
                changed = true;
            }
            if let Some(Some(target)) = targets.get(id) {
                let differs = state
                    .content_revisions
                    .get(id)
                    .is_some_and(|old| old != target)
                    || state
                        .mixed_revisions
                        .get(id)
                        .is_some_and(|versions| versions.iter().any(|old| old != target));
                if differs {
                    changed = true;
                    state.reasons.push(observation_reason(json!({"reason":"TARGET_REVISION_CHANGED","service":id,"targetRevision":target,"requiresRecheck":true})));
                }
            }
        }
        state.target_revisions = ids
            .iter()
            .map(|id| (id.clone(), targets.get(id).cloned().flatten()))
            .collect();
        if changed {
            state.reasons.push(observation_reason(
                json!({"reason":"PRIOR_RECHECK_REQUIRED","requiresRecheck":true}),
            ));
        }
        state.reasons.push(observation_reason(
            json!({"reason":"STATUS_OBSERVATION_NOT_SEMANTIC_RECHECK","requiresRecheck":true}),
        ));
        state.reasons.sort_by_key(Value::to_string);
        state.reasons.dedup();
        state.freshness = if changed {
            Freshness::Stale
        } else {
            Freshness::Unverified
        };
    }
    binding.target_revisions = targets;
    Ok(
        json!({"inputDigest":input_digest,"authority":"TARGET_OBSERVATION_NOT_SEMANTIC_RECHECK","observations":reasons}),
    )
}

pub fn refresh(repo: &Repository) -> Result<Value, ClewError> {
    let previous = bindings::baseline(repo)?
        .ok_or_else(|| invalid("no retained publication; run docs render first"))?;
    bindings::verify_outputs(repo, &previous.0, &previous.1)?;
    let previous_bytes = fs::read(repo.path("docs/index.html")?).map_err(io_error)?;
    let mut binding = previous.1.clone();
    let observation = observe(repo, &mut binding)?;
    let input_digest = observation["inputDigest"]
        .as_str()
        .ok_or_else(|| invalid("missing observed input identity"))?;
    if binding.section_states == previous.1.section_states
        && binding.target_revisions == previous.1.target_revisions
    {
        let _lock = repo.lock()?;
        if repo.input_digest()? != input_digest
            || fs::read(repo.path("docs/index.html")?).map_err(io_error)? != previous_bytes
        {
            return Err(ClewError::new(
                ErrorCode::WwConflict,
                "documentation changed during status observation",
            ));
        }
        bindings::verify_outputs(repo, &previous.0, &previous.1)?;
        return Ok(
            json!({"schema":"codeclew-docs-refresh/1.0","status":"UNCHANGED","statusOnly":true,"bundle":previous.0,"index":"docs/index.html","sections":binding.section_states,"observation":observation,"agentInvocations":0,"captures":0}),
        );
    }
    binding.schema = "codeclew-documentation-bindings/1.4".into();
    binding.output_hashes.clear();
    let bundle = digest(&json!({"binding":binding,"targetInput":input_digest,"statusRenderer":render::renderer_digest()?}))?[7..].to_owned();
    let mut files = BTreeMap::new();
    let mut cards = String::new();
    let note_targets = super::notes::snapshot(repo)?;
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
            check::PORTABLE_CACHE_MAX_BYTES,
        )?;
        attach(&mut data, subject, &binding);
        data["statusAuthority"] = json!("TARGET_OBSERVATION_NOT_SEMANTIC_RECHECK");
        if let Some(notes) = data["notes"].as_array_mut() {
            for note in notes {
                let current = note_targets.get(note["association"]["id"].as_str().unwrap_or(""));
                note["targetChanged"] =
                    json!(current.is_none_or(|current| current["associationDigest"]
                        != note["associationDigest"]
                        || current["original"] != note["original"]));
            }
        }
        for field in ["process", "view"] {
            if !data[field].is_null() {
                let stale = binding.section_states[subject].freshness == Freshness::Stale;
                data[field]["targetObservation"] = json!({
                    "authority": "TARGET_OBSERVATION_NOT_SEMANTIC_RECHECK",
                    "status": if stale { "RECHECK_REQUIRED" } else { "NOT_RECHECKED" }
                });
                if stale {
                    data[field]["targetChanged"] = json!(true);
                }
            }
        }
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
        "<!-- codeclew-bundle {bundle} -->\n<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{}</title><style>{}</style></head><body><main style=\"margin:auto;max-width:1180px;padding:32px\"><h1>{}</h1><p>Target revisions observed locally or from explicit coordinator selections. Sources and retained prose have not been rechecked.</p>{cards}</main></body></html>\n",
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
        input_digest,
        Some(&previous),
        Some(&previous_bytes),
    )?;
    Ok(
        json!({"schema":"codeclew-docs-refresh/1.0","status":"PUBLISHED","statusOnly":true,"bundle":bundle,"index":"docs/index.html","sections":sections,"observation":observation,"agentInvocations":0,"captures":0}),
    )
}
