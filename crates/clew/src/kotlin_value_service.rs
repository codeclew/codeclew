//! Retained K2 values: session admission stays in the public value service.
//! This loader binds immutable facts to their native provider and source CAS.
use crate::canonical;
use crate::cas::{CasObject, CasStore};
use crate::derived_manifest::DerivedAnalysisInputManifest;
use crate::error::{ClewError, ErrorCode};
use crate::generation_service::{AnalysisExecutionAuthority, ReadyGeneration};
use crate::generation_v2::GenerationManifest;
use crate::incremental_v2::{CompilerStoreKey, IncrementalReceipt};
use crate::java_value_service::{Projection, ValueOccurrence, ValueSource};
use crate::kotlin_adapter_v2::{
    FACT_PAYLOAD_SCHEMA, KOTLIN_FACTS_CAPABILITY, kotlin_adapter_digest,
};
use crate::kotlin_engine::{KOTLIN_ADAPTER_CONTRACT_ID, KotlinSemanticEngine};
use crate::kotlin_values::{KotlinValueProjection, POLICY};
use crate::repository_snapshot::{RepositoryInputSnapshot, WorktreeKind};
use crate::runtime::RuntimeWorker;
use clew_facts::values::{ModelBinding, ObservationAuthority};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

const MAX_BYTES: usize = 64 * 1024 * 1024;
const MAX_FACTS: u64 = 262_144;

pub(crate) fn load_projection(
    store: &CasStore,
    ready: &ReadyGeneration,
    owner: &str,
    worker: &RuntimeWorker,
) -> Result<Option<Projection>, ClewError> {
    if ready.transformed_source.is_some()
        || ready.compiler_version != "2.4.10"
        || worker.compiler_version != "2.4.10"
        || ready.incremental.analysis_execution_authority
            != AnalysisExecutionAuthority::CompilerWorker
    {
        return Err(stale("Kotlin value backend is not qualified"));
    }
    let derived: DerivedAnalysisInputManifest = read(store, &ready.derived_input_manifest)?;
    derived.verify(store)?;
    let [provider] = derived.provider_models.as_slice() else {
        return Err(corrupt("Kotlin values require one native provider"));
    };
    let [descriptor] = provider.build_model.compilations.as_slice() else {
        return Err(corrupt("Kotlin values require one compilation descriptor"));
    };
    if derived.repository_snapshot != ready.repository_snapshot
        || provider.handshake.provider_id != "project-native-kotlin"
        || provider.build_model.provider_id != "project-native-kotlin"
        || provider.handshake.provider_digest != ready.runtime_key
        || descriptor.language_uri.as_str() != "language:kotlin"
        || descriptor.source_roots.is_empty()
        || descriptor
            .source_roots
            .iter()
            .any(|s| s.tree != ready.repository_snapshot)
        || !descriptor.generated_source_roots.is_empty()
        || provider.build_model.model.object_schema != "codeclew-project-native-model/2.0"
        || descriptor.canonical_options.object_schema != "codeclew-project-native-options/3.0"
        || descriptor.toolchain.object_schema != "codeclew-kotlin-toolchain-authority/2.0"
    {
        return Err(corrupt("Kotlin value provider or source ownership differs"));
    }
    let retained_worker: RuntimeWorker = read(store, &descriptor.toolchain)?;
    if canonical::bytes(&retained_worker).map_err(internal)?
        != canonical::bytes(worker).map_err(internal)?
    {
        return Err(stale("Kotlin value worker differs from active runtime"));
    }
    let model: Value = read(store, &provider.build_model.model)?;
    let options: Value = read(store, &descriptor.canonical_options)?;
    let engine =
        serde_json::to_value(KotlinSemanticEngine::Kotlin24.authority()).map_err(internal)?;
    let semantics: crate::kotlin_engine::KotlinProjectSemantics =
        serde_json::from_value(model["projectSemantics"].clone())
            .map_err(|_| corrupt("Kotlin value project semantics are invalid"))?;
    if model["schema"] != "codeclew-project-native-model/2.0"
        || model["compilation"] != ready.compilation
        || options["nativeCompilation"] != ready.compilation
        || options["projectSemantics"] != model["projectSemantics"]
        || model["semanticEngine"] != engine
        || options["semanticEngine"] != engine
        || options["semanticAuthorityDigest"]
            != semantics.authority_digest(KotlinSemanticEngine::Kotlin24)?
    {
        return Err(corrupt(
            "Kotlin value configuration differs from compiler authority",
        ));
    }
    let receipt: IncrementalReceipt = read(store, &ready.incremental_receipt)?;
    receipt.validate()?;
    let key = CompilerStoreKey::create(
        KOTLIN_ADAPTER_CONTRACT_ID,
        kotlin_adapter_digest(&worker.tree_hash)?,
        descriptor,
    )?;
    if key.key != receipt.compiler_store_key {
        return Err(stale("Kotlin value extractor is stale"));
    }
    let generation: GenerationManifest = read(store, &ready.generation)?;
    if generation.fact_count > MAX_FACTS
        || generation
            .shards
            .iter()
            .try_fold(0u64, |n, s| n.checked_add(s.size))
            .is_none_or(|n| n > MAX_BYTES as u64)
    {
        return Err(resource("Kotlin retained value fact budget exceeded"));
    }
    let [attempt] = generation.attempts.as_slice() else {
        return Err(corrupt("Kotlin values require one compiler attempt"));
    };
    if generation.derived_input_manifest != ready.derived_input_manifest
        || generation.generation_id != receipt.generation_id
        || receipt.completeness != ready.completeness
        || attempt.compilation_id != descriptor.compilation_id
        || attempt.capability.as_str() != KOTLIN_FACTS_CAPABILITY
        || attempt.completion.fact_count != generation.fact_count
    {
        return Err(corrupt("Kotlin value generation ownership differs"));
    }
    let mut bytes_used = 0usize;
    let mut metadata = Vec::new();
    let mut files = Vec::new();
    let mut owners = Vec::new();
    generation.visit_facts(store, |fact| {
        charge(&mut bytes_used, fact.payload.size)?;
        if fact.domain_uri.as_str() != KOTLIN_FACTS_CAPABILITY
            || fact.payload.object_schema != FACT_PAYLOAD_SCHEMA
        {
            return Err(corrupt("Kotlin values include foreign fact authority"));
        }
        let payload: Value = read(store, &fact.payload)?;
        let mut parts = fact.fact_key.split(':');
        let prefix = parts.next();
        let category = parts.next();
        let digest = parts.next();
        if prefix != Some("kotlin")
            || parts.next().is_some()
            || digest
                != Some(
                    canonical::hash(&payload)
                        .map_err(internal)?
                        .trim_start_matches("sha256:"),
                )
        {
            return Err(corrupt("Kotlin value fact identity differs from payload"));
        }
        match category {
            Some("metadata") => metadata.push(payload),
            Some("file") => files.push((fact.payload.clone(), payload)),
            Some("descriptor") => {
                if payload["symbolIdentity"] == owner {
                    owners.push(payload);
                }
            }
            Some(
                "descriptor-boundary"
                | "relation"
                | "relation-boundary"
                | "local-cfg"
                | "local-cfg-boundary",
            ) => {}
            _ => return Err(corrupt("Kotlin value fact category is invalid")),
        }
        Ok(())
    })?;
    let [metadata] = metadata.as_slice() else {
        return Err(corrupt("Kotlin value metadata is ambiguous"));
    };
    let mut worker_engine = engine.clone();
    worker_engine
        .as_object_mut()
        .expect("serialized engine")
        .remove("schema");
    worker_engine
        .as_object_mut()
        .expect("serialized engine")
        .remove("runtimeName");
    let mut raw_semantics = metadata["kotlinProjectSemantics"].clone();
    let raw_options = raw_semantics["unstableCompilerOptions"]
        .as_array()
        .ok_or_else(|| corrupt("Kotlin worker compiler options are invalid"))?;
    if raw_options
        .iter()
        .any(|v| v.as_str().is_none_or(|s| !s.starts_with("-X")))
    {
        return Err(corrupt("Kotlin worker compiler options are invalid"));
    }
    raw_semantics["freeCompilerArguments"] = Value::Array(raw_options.clone());
    let worker_semantics =
        crate::kotlin_engine::KotlinProjectSemantics::from_project_model(&raw_semantics)?;
    if metadata["schema"] != "codeclew-kotlin-index-metadata/2.3"
        || metadata["compilation"] != ready.compilation
        || metadata["analyzerCompilerVersion"] != "2.4.10"
        || metadata["kotlinSemanticEngine"] != worker_engine
        || worker_semantics != semantics
        || metadata["projectCompilerVersion"] != semantics.project_compiler_version
        || metadata["projectModelHash"] != model["projectModelHash"]
        || metadata["semanticInputManifestHash"] != model["semanticInputManifestHash"]
    {
        return Err(corrupt(
            "Kotlin retained metadata differs from configuration",
        ));
    }
    for field in [
        "projectModelHash",
        "semanticInputManifestHash",
        "declarationDescriptorHash",
        "declarationRelationHash",
        "localCfgHash",
    ] {
        if !metadata[field]
            .as_str()
            .and_then(|s| s.strip_prefix("sha256:"))
            .is_some_and(|s| {
                s.len() == 64
                    && s.bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            })
        {
            return Err(corrupt(
                "Kotlin value compiler scope hash is missing or invalid",
            ));
        }
    }
    let scope = crate::kotlin_adapter_v2::semantic_scope_digest(metadata)?;
    let completion: Value = read(store, &attempt.completion.completeness_receipt)?;
    if attempt.completion.scope_digest != scope
        || attempt.completion.completeness_receipt.object_schema
            != "codeclew-completeness-receipt/2.0"
        || completion["schema"] != "codeclew-completeness-receipt/2.0"
        || completion["scopeDigest"] != scope
    {
        return Err(corrupt("Kotlin value compiler completion scope differs"));
    }
    let snapshot: RepositoryInputSnapshot = read(store, &ready.repository_snapshot)?;
    snapshot.verify()?;
    if model["snapshotId"] != snapshot.snapshot_id {
        return Err(corrupt("Kotlin value model snapshot differs"));
    }
    let mut objects: BTreeMap<_, _> = snapshot
        .index
        .iter()
        .filter(|f| f.stage == 0)
        .map(|f| (f.path.as_str(), &f.content))
        .collect();
    for entry in &snapshot.worktree {
        objects.remove(entry.path.as_str());
        if entry.kind == WorktreeKind::Regular
            && let Some(content) = &entry.content
        {
            objects.insert(entry.path.as_str(), content);
        }
    }
    let mut seen_files = BTreeSet::new();
    let mut selected = Vec::new();
    for (fact, row) in files {
        crate::kotlin_values::validate_file_fact(&row)?;
        let path = row["path"]
            .as_str()
            .filter(|p| crate::kotlin_adapter_v2::safe_relative_source_path(p))
            .ok_or_else(|| corrupt("Kotlin value file path is invalid"))?;
        if !seen_files.insert(path.to_owned()) {
            return Err(corrupt("Kotlin value source membership is ambiguous"));
        }
        let object = objects
            .get(path)
            .ok_or_else(|| corrupt("Kotlin value file is absent from retained source"))?;
        charge(&mut bytes_used, object.size)?;
        let source = store.read(object, MAX_BYTES)?;
        let digest = canonical::hash_bytes(source.bytes());
        if row["contentHash"] != digest
            || receipt
                .files
                .iter()
                .filter(|f| f.path == path && f.content_digest == digest)
                .count()
                != 1
        {
            return Err(corrupt("Kotlin value source differs from compiler inputs"));
        }
        for declaration in row["declarations"]
            .as_array()
            .ok_or_else(|| corrupt("Kotlin value file declarations are invalid"))?
        {
            if declaration["documentationSymbol"] != owner {
                continue;
            }
            let Some(value) = declaration.get("valueProjection") else {
                continue;
            };
            let projection: KotlinValueProjection = serde_json::from_value(value.clone())
                .map_err(|_| corrupt("Kotlin value projection is malformed"))?;
            selected.push((
                projection,
                fact.clone(),
                ValueSource {
                    file: path.into(),
                    content: (*object).clone(),
                    content_digest: digest.clone(),
                },
            ));
        }
    }
    if selected.is_empty() {
        return Ok(None);
    }
    let [(projection, fact, source)] = selected.as_slice() else {
        return Err(corrupt("Kotlin value owner projection is ambiguous"));
    };
    let [descriptor] = owners.as_slice() else {
        return Err(corrupt("Kotlin value compiler owner is ambiguous"));
    };
    if descriptor["declarationKind"] != "FUNCTION"
        || descriptor["file"] != source.file
        || descriptor["start"].as_u64() != Some(projection.owner_byte_start)
        || descriptor["end"].as_u64() != Some(projection.owner_byte_end)
    {
        return Err(corrupt(
            "Kotlin value projection differs from exact compiler owner",
        ));
    }
    let occurrences = source_occurrences(store, projection, source)?;
    let mut values = crate::kotlin_values::project(
        projection,
        ModelBinding {
            language: "language:kotlin".into(),
            producer: POLICY.into(),
            snapshot_digest: ready.repository_snapshot.digest.clone(),
            facts_digest: ready.generation.digest.clone(),
            configuration_digest: ready.derived_input_manifest.digest.clone(),
            policy_digest: canonical::hash_bytes(POLICY.as_bytes()),
            observation_authority: ObservationAuthority::CompilerObserved,
        },
        &fact.digest,
    )?;
    for boundary in metadata["buildModelBoundaries"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        if boundary.starts_with("KOTLIN_ANALYSIS_") {
            values.global_boundaries.push(boundary.into());
        }
    }
    values.global_boundaries.sort();
    values.global_boundaries.dedup();
    values
        .validate()
        .map_err(|_| corrupt("Kotlin value model boundary is invalid"))?;
    Ok(Some((values, vec![source.clone()], occurrences, None)))
}

fn source_occurrences(
    store: &CasStore,
    p: &KotlinValueProjection,
    source: &ValueSource,
) -> Result<Vec<ValueOccurrence>, ClewError> {
    let lease = store.read(&source.content, MAX_BYTES)?;
    let text = std::str::from_utf8(lease.bytes())
        .map_err(|_| corrupt("Kotlin value source is not UTF-8"))?;
    validate_coordinates(p, text)?;
    Ok(p.nodes
        .iter()
        .map(|n| ValueOccurrence {
            definition: n.id.clone(),
            owner: p.owner.clone(),
            file: n.file.clone(),
            byte_start: n.byte_start,
            byte_end: n.byte_end,
            start_line: n.start_line,
            end_line: n.end_line,
        })
        .collect())
}

fn validate_coordinates(p: &KotlinValueProjection, text: &str) -> Result<(), ClewError> {
    if p.full_compilation_source_digest != canonical::hash_bytes(text.as_bytes())
        || text
            .get(p.owner_byte_start as usize..p.owner_byte_end as usize)
            .is_none_or(str::is_empty)
    {
        return Err(corrupt("Kotlin value owner escapes retained source"));
    }
    let mut wanted = BTreeSet::new();
    for n in &p.nodes {
        let start = usize::try_from(n.byte_start)
            .map_err(|_| corrupt("Kotlin value coordinate exceeds host size"))?;
        let end = usize::try_from(n.byte_end)
            .map_err(|_| corrupt("Kotlin value coordinate exceeds host size"))?;
        if text.get(start..end).is_none_or(str::is_empty) {
            return Err(corrupt(
                "Kotlin value occurrence escapes retained UTF-8 source",
            ));
        }
        wanted.insert(start);
        wanted.insert(end);
    }
    // K2 normalizes CR/CRLF before assigning UTF-16 coordinates. Original byte
    // and line anchors still describe the retained, unnormalized UTF-8 text.
    let mut positions = BTreeMap::new();
    let (mut compiler_offset, mut line) = (0u64, 1u64);
    let mut previous_cr = false;
    for (offset, ch) in text
        .char_indices()
        .chain(std::iter::once((text.len(), '\0')))
    {
        if wanted.contains(&offset) {
            if (offset == 0 && ch == '\u{feff}') || (previous_cr && ch == '\n') {
                return Err(corrupt(
                    "Kotlin value occurrence points inside normalized compiler input",
                ));
            }
            positions.insert(offset, (compiler_offset, line));
        }
        if !(previous_cr && ch == '\n' || offset == 0 && ch == '\u{feff}') {
            compiler_offset += ch.len_utf16() as u64;
        }
        if ch == '\n' {
            line += 1;
        }
        previous_cr = ch == '\r';
    }
    for n in &p.nodes {
        let (start16, start_line) = positions[&(n.byte_start as usize)];
        let (end16, end_line) = positions[&(n.byte_end as usize)];
        let collapsed = n.id == "result"
            && matches!(&n.operation, clew_facts::values::ValueOperation::Unknown {boundary} if boundary == "KOTLIN_VALUE_BODY_OR_BUDGET_UNSUPPORTED");
        let inclusive_end = end_line - u64::from(text.as_bytes()[n.byte_end as usize - 1] == b'\n');
        if n.start != start16
            || n.end != end16
            || n.start_line != start_line
            || (n.end_line != end_line && !(collapsed && n.end_line == inclusive_end))
        {
            return Err(corrupt(
                "Kotlin value occurrence differs from compiler/source coordinates",
            ));
        }
    }
    Ok(())
}

fn read<T: DeserializeOwned + Serialize>(
    store: &CasStore,
    object: &CasObject,
) -> Result<T, ClewError> {
    let lease = store.read(object, MAX_BYTES)?;
    let value: T = serde_json::from_slice(lease.bytes())
        .map_err(|_| corrupt("Kotlin retained value object is malformed"))?;
    if canonical::bytes(&value).map_err(internal)? != lease.bytes() {
        return Err(corrupt("Kotlin retained value object is not canonical"));
    }
    Ok(value)
}
fn charge(total: &mut usize, size: u64) -> Result<(), ClewError> {
    *total = total
        .checked_add(
            usize::try_from(size)
                .map_err(|_| resource("Kotlin value closure exceeds host size"))?,
        )
        .filter(|n| *n <= MAX_BYTES)
        .ok_or_else(|| resource("Kotlin value source/fact closure exceeds budget"))?;
    Ok(())
}
fn corrupt(s: &str) -> ClewError {
    ClewError::new(ErrorCode::StateCorrupt, s)
}
fn stale(s: &str) -> ClewError {
    ClewError::new(ErrorCode::ProjectModelChanged, s)
}
fn resource(s: &str) -> ClewError {
    ClewError::new(ErrorCode::ResourceLimit, s)
}
fn internal(e: impl std::fmt::Display) -> ClewError {
    ClewError::new(ErrorCode::Internal, e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter_v2::*;
    use crate::generation_service::{
        IncrementalExecutionEvidence, IncrementalExecutionMode, READY_GENERATION_SCHEMA,
    };
    use crate::generation_v2::{AttemptAuthority, FactRunWriter, finalize_generation};
    use crate::incremental_v2::{
        CompletenessVector, FileReceipt, FullAnalysisReason, INCREMENTAL_RECEIPT_SCHEMA,
        IncrementalPlan,
    };
    use crate::state::StateAuthority;
    use crate::thread_values::{Nullness, ValueQuery};
    use crate::worker::WorkerRequestCounters;
    use serde_json::json;
    use std::{fs, process::Command};

    fn golden() -> Vec<KotlinValueProjection> {
        serde_json::from_str(include_str!(
            "../../../fixtures/kotlin-values/projections.json"
        ))
        .unwrap()
    }

    #[test]
    fn source_coordinates_match_native_golden_and_normalized_unicode_text() {
        let source = include_str!("../../../fixtures/kotlin-values/Values.kt");
        for p in golden() {
            validate_coordinates(&p, source).unwrap();
        }
        let mut p = golden()
            .into_iter()
            .find(|p| p.owner.contains(".nullValue#"))
            .unwrap();
        let original = p.clone();
        let map = |offset: u64| {
            offset
                + 3
                + source[..offset as usize]
                    .bytes()
                    .filter(|b| *b == b'\n')
                    .count() as u64
        };
        let normalized = format!("\u{feff}{}", source.replace('\n', "\r\n"));
        p.owner_byte_start = map(p.owner_byte_start);
        p.owner_byte_end = map(p.owner_byte_end);
        for n in &mut p.nodes {
            n.byte_start = map(n.byte_start);
            n.byte_end = map(n.byte_end);
        }
        p.full_compilation_source_digest = canonical::hash_bytes(normalized.as_bytes());
        validate_coordinates(&p, &normalized).unwrap();
        p.nodes[0].start += 1;
        assert!(validate_coordinates(&p, &normalized).is_err());
        assert!(validate_coordinates(&original, &normalized).is_err());
    }

    // Structural binding fixture only. Native compiler/public session evidence
    // is established separately; no test JSON authenticates a compiler run.
    fn fixture(
        change: impl FnOnce(&mut Value),
    ) -> (
        tempfile::TempDir,
        CasStore,
        ReadyGeneration,
        RuntimeWorker,
        String,
    ) {
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("repo");
        fs::create_dir(&repo).unwrap();
        let source_path = "fixtures/kotlin-values/Values.kt";
        fs::create_dir_all(repo.join("fixtures/kotlin-values")).unwrap();
        let source = include_str!("../../../fixtures/kotlin-values/Values.kt");
        fs::write(repo.join(source_path), source).unwrap();
        for args in [
            vec!["init", "-q"],
            vec!["add", "."],
            vec![
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "-c",
                "core.hooksPath=/dev/null",
                "commit",
                "-qm",
                "fixture",
            ],
        ] {
            assert!(
                Command::new("git")
                    .args(args)
                    .current_dir(&repo)
                    .output()
                    .unwrap()
                    .status
                    .success()
            );
        }
        let state = StateAuthority::open(root.path().join("state")).unwrap();
        let store = CasStore::open(&state).unwrap();
        let (snapshot, snapshot_object) =
            crate::repository_snapshot::capture(&repo, &store).unwrap();
        let p = golden()
            .into_iter()
            .find(|p| p.owner.contains(".nullValue#"))
            .unwrap();
        let owner = p.owner.clone();
        let worker = RuntimeWorker {
            protocol: crate::runtime::KOTLIN_WORKER_PROTOCOL.into(),
            compiler_version: "2.4.10".into(),
            distribution: "test".into(),
            tree_hash: canonical::hash_bytes(b"fixture-worker"),
            files: vec![],
        };
        let semantics = crate::kotlin_engine::KotlinProjectSemantics::from_project_model(
            &json!({"compilerVersion":"2.4.10"}),
        )
        .unwrap();
        let engine = KotlinSemanticEngine::Kotlin24.authority();
        let model_hash = canonical::hash_bytes(b"fixture-model");
        let input_hash = canonical::hash_bytes(b"fixture-input");
        let model = store.put("codeclew-project-native-model/2.0",&canonical::bytes(&json!({"schema":"codeclew-project-native-model/2.0","snapshotId":snapshot.snapshot_id,"compilation":":/main","projectSemantics":semantics,"semanticEngine":engine,"projectModelHash":model_hash,"semanticInputManifestHash":input_hash})).unwrap()).unwrap();
        let descriptor = CompilationDescriptor {schema:COMPILATION_SCHEMA.into(),compilation_id:"main".into(),language_uri:LanguageUri::parse("language:kotlin").unwrap(),source_roots:vec![SourceRootDescriptor{logical_name:"project".into(),tree:snapshot_object.clone()}],generated_source_roots:vec![],classpath:vec![],toolchain:store.put("codeclew-kotlin-toolchain-authority/2.0",&canonical::bytes(&worker).unwrap()).unwrap(),plugins:vec![],canonical_options:store.put("codeclew-project-native-options/3.0",&canonical::bytes(&json!({"nativeCompilation":":/main","projectSemantics":semantics,"semanticEngine":engine,"semanticAuthorityDigest":semantics.authority_digest(KotlinSemanticEngine::Kotlin24).unwrap()})).unwrap()).unwrap(),dependency_compilation_ids:vec![],operations:vec![],origin:DescriptorOrigin::ProjectNative,completeness:DescriptorCompleteness::Unknown};
        let runtime_key = canonical::hash_bytes(b"fixture-runtime");
        let (_, derived) = DerivedAnalysisInputManifest::create(
            &store,
            snapshot_object.clone(),
            vec![ProviderModel {
                handshake: ProviderHandshake {
                    protocol: PROVIDER_PROTOCOL.into(),
                    provider_id: "project-native-kotlin".into(),
                    provider_digest: runtime_key.clone(),
                    build_system_uris: vec!["build:project-native".into()],
                },
                build_model: BuildModel {
                    provider_id: "project-native-kotlin".into(),
                    model,
                    compilations: vec![descriptor.clone()],
                },
            }],
        )
        .unwrap();
        let mut raw_engine = serde_json::to_value(engine).unwrap();
        raw_engine.as_object_mut().unwrap().remove("schema");
        raw_engine.as_object_mut().unwrap().remove("runtimeName");
        let mut raw_semantics = serde_json::to_value(semantics).unwrap();
        raw_semantics.as_object_mut().unwrap().remove("schema");
        let mut index = json!({"compilation":":/main","compilerVersion":"2.4.10","projectCompilerVersion":"2.4.10","analyzerCompilerVersion":"2.4.10","kotlinProjectSemantics":raw_semantics,"kotlinSemanticEngine":raw_engine,"buildModelBoundaries":[],"projectModelHash":model_hash,"semanticInputManifestHash":input_hash,"files":[{"path":source_path,"contentHash":canonical::hash_bytes(source.as_bytes()),"declarations":[{"documentationSymbol":owner,"valueProjection":p}]}],"declarationDescriptors":{"descriptors":[{"declarationKind":"FUNCTION","symbolIdentity":owner,"file":source_path,"start":p.owner_byte_start,"end":p.owner_byte_end}],"boundaries":[]},"declarationRelations":{"relations":[],"boundaries":[]},"localCfgs":[],"localCfgBoundaries":[]});
        index["declarationDescriptorHash"] =
            json!(canonical::hash(&index["declarationDescriptors"]).unwrap());
        index["declarationRelationHash"] =
            json!(canonical::hash(&index["declarationRelations"]).unwrap());
        index["localCfgHash"] = json!(canonical::hash(&index["localCfgs"]).unwrap());
        change(&mut index);
        let facts = crate::kotlin_adapter_v2::translate_facts(&store, &index).unwrap();
        let count = facts.len() as u64;
        let scope = crate::kotlin_adapter_v2::semantic_scope_digest(&index).unwrap();
        let complete = CompletenessVector::verified_complete(scope.clone()).unwrap();
        let mut writer = FactRunWriter::create(&state).unwrap();
        for fact in facts {
            writer.push(&fact).unwrap();
        }
        let (generation, generation_object) = finalize_generation(
            &store,
            derived.clone(),
            vec![AttemptAuthority {
                compilation_id: "main".into(),
                capability: CapabilityUri::parse(KOTLIN_FACTS_CAPABILITY).unwrap(),
                completion: AnalysisAttemptComplete {
                    scope_digest: scope.clone(),
                    completeness_receipt: store.put("codeclew-completeness-receipt/2.0", &canonical::bytes(&json!({"schema":"codeclew-completeness-receipt/2.0","scopeDigest":scope,"domains":[{"domain":KOTLIN_FACTS_CAPABILITY,"support":"SUPPORTED","coverage":"COMPLETE","certainty":"VERIFIED"}],"obligations":[]})).unwrap()).unwrap(),
                    fact_count: count,
                },
            }],
            vec![writer.finish().unwrap()],
        )
        .unwrap();
        let receipt = IncrementalReceipt {
            schema: INCREMENTAL_RECEIPT_SCHEMA.into(),
            compiler_store_key: CompilerStoreKey::create(
                KOTLIN_ADAPTER_CONTRACT_ID,
                kotlin_adapter_digest(&worker.tree_hash).unwrap(),
                &descriptor,
            )
            .unwrap()
            .key,
            generation_id: generation.generation_id,
            files: vec![FileReceipt {
                path: source_path.into(),
                content_digest: canonical::hash_bytes(source.as_bytes()),
                exported_surface_digest: canonical::hash_bytes(b"fixture-surface"),
                dependencies: vec![],
            }],
            boundaries: vec![],
            completeness: complete.clone(),
        };
        let ready = ReadyGeneration {
            schema: READY_GENERATION_SCHEMA.into(),
            generation_key: canonical::hash_bytes(b"fixture"),
            runtime_key,
            base_revision: "1".repeat(40),
            compilation: ":/main".into(),
            compiler_version: "2.4.10".into(),
            completeness: complete,
            coverage: "COMPLETE".into(),
            certainty: "VERIFIED".into(),
            obligations: vec![],
            repository_snapshot: snapshot_object,
            derived_input_manifest: derived,
            generation: generation_object,
            incremental_receipt: store
                .put(
                    INCREMENTAL_RECEIPT_SCHEMA,
                    &canonical::bytes(&receipt).unwrap(),
                )
                .unwrap(),
            query_index: store.put("test/query", b"unused").unwrap(),
            transformed_source: None,
            incremental: IncrementalExecutionEvidence {
                schema: "codeclew-incremental-execution/3.0".into(),
                planned: IncrementalPlan::Full {
                    reason: FullAnalysisReason::NoParent,
                },
                executed: IncrementalExecutionMode::Full,
                analysis_execution_authority: AnalysisExecutionAuthority::CompilerWorker,
                compiler_output_receipt: None,
                subset_analysis_supported: false,
                worker_requests: WorkerRequestCounters {
                    open_project_requests: 1,
                    index_files_requests: 1,
                },
            },
        };
        (root, store, ready, worker, owner)
    }

    #[test]
    fn retained_projection_binds_exact_owner_source_generation_and_runtime() {
        let (root, store, mut ready, worker, owner) = fixture(|_| {});
        let (model, sources, occurrences, calls) = load_projection(&store, &ready, &owner, &worker)
            .unwrap()
            .unwrap();
        let q = ValueQuery {
            target: "result".into(),
            max_nodes: 128,
            max_transfers: 4096,
        };
        let result = crate::thread_values::analyze(&model, &q).unwrap();
        assert_eq!(
            result
                .observations
                .iter()
                .find(|v| v.id == "result")
                .unwrap()
                .normal_value_nullness,
            Nullness::Null
        );
        assert_eq!(sources.len(), 1);
        assert_eq!(occurrences.len(), 2);
        assert!(calls.is_none());
        fs::write(
            root.path().join("repo/fixtures/kotlin-values/Values.kt"),
            "unrelated live source",
        )
        .unwrap();
        assert_eq!(
            load_projection(&store, &ready, &owner, &worker)
                .unwrap()
                .unwrap()
                .0,
            model
        );
        assert!(
            load_projection(
                &store,
                &ready,
                "callable:values/Values.nullValue#jvm:()I",
                &worker
            )
            .unwrap()
            .is_none()
        );
        let mut other_worker = worker.clone();
        other_worker.tree_hash = canonical::hash_bytes(b"other-worker");
        assert_eq!(
            load_projection(&store, &ready, &owner, &other_worker)
                .unwrap_err()
                .code,
            ErrorCode::ProjectModelChanged
        );
        ready.compilation = ":other/main".into();
        assert!(load_projection(&store, &ready, &owner, &worker).is_err());
    }

    #[test]
    fn foreign_metadata_duplicate_owner_and_tampered_source_coordinates_reject() {
        for change in [0, 1, 2, 3, 4, 5] {
            let (_root, store, ready, worker, owner) = fixture(|index| match change {
                0 => index["projectModelHash"] = json!(canonical::hash_bytes(b"foreign-model")),
                1 => {
                    let duplicate = index["files"][0]["declarations"][0].clone();
                    index["files"][0]["declarations"]
                        .as_array_mut()
                        .unwrap()
                        .push(duplicate);
                }
                2 => {
                    index["files"][0]["declarations"][0]["valueProjection"]["nodes"][0]["startLine"] =
                        json!(99);
                    index["files"][0]["declarations"][0]["valueProjection"]["nodes"][0]["endLine"] =
                        json!(99);
                }
                3 => index["declarationDescriptors"]["descriptors"][0]["start"] = json!(0),
                4 => {
                    index
                        .as_object_mut()
                        .unwrap()
                        .remove("declarationDescriptorHash");
                }
                _ => index["declarationRelationHash"] = Value::Null,
            });
            assert!(
                load_projection(&store, &ready, &owner, &worker).is_err(),
                "change {change}"
            );
        }
    }

    #[test]
    fn foreign_completion_scope_and_receipt_reject_even_with_same_fact_count() {
        for change_receipt in [false, true] {
            let (root, store, mut ready, worker, owner) = fixture(|_| {});
            let original: GenerationManifest = read(&store, &ready.generation).unwrap();
            let mut attempts = original.attempts.clone();
            if change_receipt {
                let mut completion: Value =
                    read(&store, &attempts[0].completion.completeness_receipt).unwrap();
                completion["scopeDigest"] = json!(canonical::hash_bytes(b"foreign-completion"));
                attempts[0].completion.completeness_receipt = store
                    .put(
                        "codeclew-completeness-receipt/2.0",
                        &canonical::bytes(&completion).unwrap(),
                    )
                    .unwrap();
            } else {
                attempts[0].completion.scope_digest = canonical::hash_bytes(b"foreign-scope");
            }
            let state = StateAuthority::open(root.path().join("state")).unwrap();
            let mut writer = FactRunWriter::create(&state).unwrap();
            original
                .visit_facts(&store, |fact| writer.push(fact))
                .unwrap();
            let (generation, object) = finalize_generation(
                &store,
                ready.derived_input_manifest.clone(),
                attempts,
                vec![writer.finish().unwrap()],
            )
            .unwrap();
            assert_eq!(generation.fact_count, original.fact_count);
            let mut receipt: IncrementalReceipt = read(&store, &ready.incremental_receipt).unwrap();
            receipt.generation_id = generation.generation_id;
            ready.incremental_receipt = store
                .put(
                    INCREMENTAL_RECEIPT_SCHEMA,
                    &canonical::bytes(&receipt).unwrap(),
                )
                .unwrap();
            ready.generation = object;
            assert!(
                load_projection(&store, &ready, &owner, &worker)
                    .unwrap_err()
                    .message
                    .contains("completion scope")
            );
        }
    }
}
