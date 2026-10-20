//! Inventory receipts from admitted inputs, independent of emitted declarations.
//! The Java source audit supplies possible surfaces, not runtime registrations.
use super::{analysis, invalid, io_error, model::*, syntax};
use crate::{
    cas::{CasObject, CasStore},
    error::ClewError,
    generation_service::ReadyGenerationSet,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

fn surface(
    e: &mut ServiceEvidence,
    symbol: &str,
    normalized: Value,
    sources: Vec<String>,
) -> Result<(), ClewError> {
    syntax::observe(e, "ENTRY_SURFACE", symbol, normalized, sources)?;
    Ok(())
}

/// Conservative unsupported composed-annotation route: only annotated callables
/// inside an explicitly declared Spring controller, never all public helpers.
pub(super) fn attach_syntax_surfaces(e: &mut ServiceEvidence) -> Result<(), ClewError> {
    let mut pending = Vec::new();
    for o in e
        .observations
        .values()
        .filter(|o| o.kind == "SOURCE_ANNOTATIONS")
    {
        let facts = &o.normalized["facts"];
        let controller = facts["owners"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|owner| {
                owner["annotations"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|a| {
                        matches!(
                            a["qualifiedName"].as_str(),
                            Some(
                                "org.springframework.web.bind.annotation.RestController"
                                    | "org.springframework.stereotype.Controller"
                            )
                        )
                    })
            });
        let annotations: Vec<_> = facts["declaration"]["annotations"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|a| {
                a["qualifiedName"].as_str().is_none_or(|name| {
                    !name.starts_with("java.") && !name.starts_with("org.springframework.")
                })
            })
            .cloned()
            .collect();
        let mapped = o.normalized["framework"]["entries"]
            .as_array()
            .is_some_and(|a| !a.is_empty());
        if controller && !mapped && !annotations.is_empty() {
            pending.push((o.symbol.clone(),json!({"authority":"SOURCE_SURFACE_NOT_REGISTRATION","name":e.observations.values().find(|d|d.kind=="SYMBOL"&&d.symbol==o.symbol).map(|d|&d.normalized["name"]),"annotations":annotations,
                "boundary":"CUSTOM_CONTROLLER_ANNOTATION_NOT_INTERPRETED","nextAction":"read annotation declaration and controller method; retain possible composed mapping until framework interpretation is supported"}),o.source_ids.clone()));
        }
    }
    for (symbol, record, sources) in pending {
        surface(e, &symbol, record, sources)?;
    }
    Ok(())
}

/// The compiler audit parses the same immutable input bytes solely for discovery
/// diagnostics. Its unscoped syntax symbols/flow never enter compiler evidence.
pub(super) fn attach_java_inputs(
    e: &mut ServiceEvidence,
    service: &Service,
    scope: &str,
    inputs: &BTreeMap<String, Arc<str>>,
    excluded: &BTreeSet<String>,
    authority: &str,
) -> Result<(), ClewError> {
    let mut inventory = BTreeMap::new();
    let mut boundaries = Vec::new();
    let mut capture_failures = Vec::new();
    let mut audited_bytes = 0usize;
    for (index, (path, text)) in inputs.iter().enumerate() {
        let audit = if index < 2048 && audited_bytes.saturating_add(text.len()) <= 32 * 1024 * 1024
        {
            audited_bytes += text.len();
            syntax::audit_java_file(service, path, text, &e.revision)
        } else {
            Err(invalid("ENTRY_SURFACE_AUDIT_BUDGET"))
        };
        let mut audit = match audit {
            Ok(audit) => audit,
            Err(_) => {
                inventory.insert(path.clone(),json!({"digest":crate::canonical::hash_bytes(text.as_bytes()),"coverage":"AUDIT_UNAVAILABLE"}));
                boundaries.push(format!("ENTRY_SURFACE_AUDIT_UNAVAILABLE:{path}"));
                continue;
            }
        };
        attach_syntax_surfaces(&mut audit)?;
        inventory.insert(path.clone(),json!({"digest":crate::canonical::hash_bytes(text.as_bytes()),"coverage":audit.coverage}));
        boundaries.extend(audit.boundaries.iter().map(|b| format!("{scope}:{b}")));
        if audit.coverage == "PARTIAL" {
            capture_failures.extend(
                audit
                    .boundaries
                    .iter()
                    .filter(|b| b.starts_with("PARSE_ERROR:"))
                    .map(|b| format!("{scope}:{b}")),
            );
        }
        let mut records:Vec<_>=audit.entrypoints.iter().filter(|entry|entry.kind!="SOURCE_DECLARATION").map(|entry|(format!("{}:{scope}",entry.id),json!({"authority":"SOURCE_SURFACE_NOT_REGISTRATION","scope":scope,"symbol":entry.symbol,"kind":entry.kind,"trigger":entry.trigger,"boundaries":entry.boundaries,"nextAction":"compare source mapping with compiler-discovered entry; inspect inherited registration if absent"}),entry.source_ids.clone())).collect();
        records.extend(
            audit
                .observations
                .values()
                .filter(|o| o.kind == "ENTRY_SURFACE")
                .map(|o| {
                    (
                        format!("{}:{scope}", o.id),
                        o.normalized.clone(),
                        o.source_ids.clone(),
                    )
                }),
        );
        for (symbol, mut record, source_ids) in records {
            record["scope"] = json!(scope);
            let mut ids = Vec::new();
            for id in source_ids {
                let mut source = audit.sources[&id].clone();
                source.id = analysis::source_id(&e.service, &format!("audit:{scope}:{id}"))?;
                source.authority = "ADMITTED_COMPILATION_SOURCE_AUDIT_NOT_CALL_RESOLUTION".into();
                // These can be sealed transformed inputs, not bytes in the
                // repository revision. Bind the admitted input receipt without
                // inventing a Git occurrence or a remote line URL.
                source.occurrence = None;
                source.url = None;
                source.evidence_digest = super::digest(&(
                    authority,
                    scope,
                    &source.file,
                    source.start_line,
                    source.end_line,
                    &source.text_digest,
                ))?;
                ids.push(source.id.clone());
                e.sources.insert(source.id.clone(), source);
            }
            surface(e, &symbol, record, ids)?;
        }
    }
    if inputs.is_empty() {
        capture_failures.push("EMPTY_SELECTED_COMPILATION_INPUTS".into());
    }
    boundaries.sort();
    boundaries.dedup();
    let normalized = json!({"schema":"codeclew-capture-scope/1.0","authority":authority,"compilation":scope,"inventory":inventory,"excludedFiles":excluded,
        "boundaries":boundaries,"captureFailures":capture_failures,"frameworkForms":"JAVA_SPRING_DIRECT_MAPPINGS_WITH_POSSIBLE_INHERITED_AND_CUSTOM_SURFACES","runtimeRegistration":"NOT_ESTABLISHED"});
    syntax::observe(
        e,
        "CAPTURE_SCOPE",
        &format!("capture-scope:{scope}"),
        normalized,
        vec![],
    )?;
    if !capture_failures.is_empty() {
        e.coverage = "PARTIAL".into();
        e.boundaries.extend(capture_failures);
    }
    Ok(())
}

pub(super) fn attach_compiler(
    e: &mut ServiceEvidence,
    service: &Service,
    ready: &ReadyGenerationSet,
    store: &CasStore,
    original: &BTreeMap<String, Arc<str>>,
) -> Result<(), ClewError> {
    if service.language != "java" {
        return Ok(());
    }
    for compilation in &ready.compilations {
        let lease = store.read(&compilation.derived_input_manifest, 16 * 1024 * 1024)?;
        let manifest: crate::derived_manifest::DerivedAnalysisInputManifest =
            serde_json::from_slice(lease.bytes()).map_err(io_error)?;
        let mut selected = None;
        for descriptor in manifest
            .provider_models
            .iter()
            .flat_map(|p| &p.build_model.compilations)
        {
            if descriptor.language_uri.as_str() != "language:java" {
                continue;
            }
            let lease = store.read(&descriptor.canonical_options, 16 * 1024 * 1024)?;
            let options: Value = serde_json::from_slice(lease.bytes()).map_err(io_error)?;
            let model: crate::java_project_model::JavaProjectModel =
                if descriptor.canonical_options.object_schema
                    == crate::java_project_model::JAVA_MODEL_SCHEMA
                {
                    serde_json::from_value(options).map_err(io_error)?
                } else {
                    let reference: CasObject =
                        serde_json::from_value(options["model"].clone()).map_err(io_error)?;
                    let lease = store.read(&reference, 16 * 1024 * 1024)?;
                    serde_json::from_slice(lease.bytes()).map_err(io_error)?
                };
            if model.compilation == compilation.compilation {
                selected = Some(model);
                break;
            }
        }
        let model = selected.ok_or_else(|| {
            invalid("Java thread inventory has no admitted model for selected compilation")
        })?;
        let transformed = compilation
            .transformed_source
            .as_ref()
            .map(|r| crate::generation_service::load_transformed_source(store, r))
            .transpose()?;
        let mut inputs = BTreeMap::new();
        for path in &model.source_files {
            let text = if let Some(table) = &transformed {
                table
                    .get(path)
                    .and_then(|b| std::str::from_utf8(b).ok())
                    .map(Arc::<str>::from)
            } else {
                original.get(path).cloned()
            };
            inputs.insert(
                path.clone(),
                text.ok_or_else(|| {
                    invalid(format!(
                        "thread inventory admitted input unavailable: {}",
                        path
                    ))
                })?,
            );
        }
        let selected: BTreeSet<_> = inputs.keys().cloned().collect();
        let excluded = original
            .keys()
            .filter(|p| p.ends_with(".java") && !selected.contains(*p))
            .cloned()
            .collect();
        attach_java_inputs(
            e,
            service,
            &compilation.compilation,
            &inputs,
            &excluded,
            &super::digest(&(
                &compilation.derived_input_manifest,
                &compilation.transformed_source,
            ))?,
        )?;
    }
    Ok(())
}
