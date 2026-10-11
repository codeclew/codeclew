//! Read-only value queries over authenticated, immutable Java generations.
//! No caller-supplied index or model JSON enters this publication contour.
use crate::canonical;
use crate::cas::{CasObject, CasStore};
use crate::derived_manifest::DerivedAnalysisInputManifest;
use crate::error::{ClewError, ErrorCode};
use crate::generation_service::{ReadyGeneration, load_session_generation};
use crate::generation_v2::GenerationManifest;
use crate::incremental_v2::{CompilerStoreKey, IncrementalReceipt};
use crate::java_adapter_v2::{
    JAVA_ANALYZER_SOURCE, JAVA_COMPILER_FACTS_CAPABILITY, JAVA_FACT_SCHEMA, JAVA_INDEX_SCHEMA,
    JavaCompilerFact, JavaCompilerIndex, java_adapter_digest, java_scope_digest,
};
use crate::java_project_model::{JAVA_MODEL_SCHEMA, JavaProjectModel};
use crate::repository_snapshot::{RepositoryInputSnapshot, WorktreeKind};
use crate::session::{SessionAuthority, SessionLanguage};
use crate::state::StateAuthority;
use crate::thread_values::{ValueQuery, ValueResult};
use clew_facts::values::{ValueModel, ValueNode};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::collections::BTreeMap;

pub const RESULT_SCHEMA: &str = "codeclew-retained-value-query/0.1";
const MAX_CLOSURE_BYTES: usize = 64 * 1024 * 1024;
const MAX_FACTS: u64 = 262_144;
const MAX_STDOUT_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ValueSource {
    pub file: String,
    pub content: CasObject,
    pub content_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ValueOccurrence {
    pub definition: String,
    pub file: String,
    pub byte_start: u64,
    pub byte_end: u64,
    pub start_line: u64,
    pub end_line: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RetainedValueQuery {
    pub schema: String,
    pub status: String,
    pub observation_authority: String,
    pub session_id: String,
    pub session_authority_digest: String,
    pub compilation: String,
    pub owner: String,
    pub query: ValueQuery,
    pub program_semantic_closure: bool,
    pub unknown_codes: Vec<String>,
    pub generation: Option<CasObject>,
    pub configuration: Option<CasObject>,
    pub sources: Vec<ValueSource>,
    pub occurrences: Vec<ValueOccurrence>,
    pub model_digest: Option<String>,
    pub retained_definition_count: usize,
    /// Selected equations only; cut operands remain explicit in result.frontier.
    /// This is a packet projection, never a new independently valid ValueModel.
    pub equations: Vec<ValueNode>,
    pub result: Option<ValueResult>,
}

/// Read the already saved generation. Never run a compiler, recapture a
/// checkout, publish another root, or compare historical bytes with live HEAD.
pub fn query(
    session_id: &str,
    compilation: &str,
    owner: &str,
    query: ValueQuery,
) -> Result<RetainedValueQuery, ClewError> {
    for text in [compilation, owner, &query.target] {
        if text.is_empty() || text.len() > 1024 || text.chars().any(char::is_control) {
            return Err(ClewError::new(
                ErrorCode::InvalidInput,
                "invalid retained value selector",
            ));
        }
    }
    if query.max_nodes == 0
        || query.max_nodes > clew_facts::values::MAX_NODES
        || query.max_transfers == 0
        || query.max_transfers > 1_000_000
    {
        return Err(ClewError::new(
            ErrorCode::InvalidInput,
            "invalid retained value budget",
        ));
    }
    let (session, _) = SessionAuthority::load(session_id)?;
    session.require_open()?;
    let mut answer = RetainedValueQuery {
        schema: RESULT_SCHEMA.into(),
        status: "UNKNOWN".into(),
        observation_authority: "NONE".into(),
        session_id: session.session_id.clone(),
        session_authority_digest: session.authority_digest.clone(),
        compilation: compilation.into(),
        owner: owner.into(),
        query,
        program_semantic_closure: false,
        unknown_codes: Vec::new(),
        generation: None,
        configuration: None,
        sources: Vec::new(),
        occurrences: Vec::new(),
        model_digest: None,
        retained_definition_count: 0,
        equations: Vec::new(),
        result: None,
    };
    if session.language != SessionLanguage::Java {
        answer
            .unknown_codes
            .push("VALUE_BACKEND_NOT_QUALIFIED".into());
        return Ok(answer);
    }
    if !session.compilations.iter().any(|c| c == compilation) {
        answer.unknown_codes.push("COMPILATION_NOT_SELECTED".into());
        return Ok(answer);
    }
    let ready = match load_session_generation(&session) {
        Ok(ready) => ready,
        Err(error) if error.code == ErrorCode::ResourceLimit => return Err(error),
        Err(_) => {
            answer
                .unknown_codes
                .push("RETAINED_GENERATION_UNAVAILABLE_OR_INVALID".into());
            return Ok(answer);
        }
    };
    let selected = ready
        .compilations
        .iter()
        .find(|c| c.compilation == compilation)
        .ok_or_else(|| corrupt("selected Java compilation is absent"))?;
    if selected.transformed_source.is_some() {
        answer
            .unknown_codes
            .push("JAVA_VALUE_TRANSFORMED_SOURCE_NOT_QUALIFIED".into());
        return Ok(answer);
    }
    let state = StateAuthority::process_default()?;
    let store = CasStore::open(&state)?;
    let projection = match load_projection(&store, selected, owner) {
        Ok(projection) => projection,
        Err(error) if error.code == ErrorCode::ProjectModelChanged => {
            answer
                .unknown_codes
                .push("JAVA_VALUE_PROJECTION_STALE".into());
            return Ok(answer);
        }
        Err(error) => return Err(error),
    };
    let Some((model, sources, occurrences)) = projection else {
        answer
            .unknown_codes
            .push("JAVA_VALUE_OWNER_OR_PROJECTION_UNAVAILABLE".into());
        return Ok(answer);
    };
    let result = crate::thread_values::analyze(&model, &answer.query)
        .map_err(|message| ClewError::new(ErrorCode::InvalidInput, message))?;
    if result
        .observations
        .iter()
        .any(|value| value.id == answer.query.target)
    {
        answer.status = "AVAILABLE".into();
    } else {
        answer.unknown_codes.push("VALUE_NOT_FOUND_UNKNOWN".into());
    }
    answer.observation_authority = "AUTHENTICATED_RETAINED_COMPILER_PROJECTION".into();
    answer.generation = Some(selected.generation.clone());
    answer.configuration = Some(selected.derived_input_manifest.clone());
    let selected: std::collections::BTreeSet<_> = result
        .observations
        .iter()
        .map(|value| value.id.as_str())
        .collect();
    answer.occurrences = occurrences
        .into_iter()
        .filter(|occurrence| selected.contains(occurrence.definition.as_str()))
        .collect();
    answer.sources = sources
        .into_iter()
        .filter(|source| {
            answer
                .occurrences
                .iter()
                .any(|occurrence| occurrence.file == source.file)
        })
        .collect();
    answer.retained_definition_count = model.nodes.len();
    answer.equations = model
        .nodes
        .into_iter()
        .filter(|node| selected.contains(node.id.as_str()))
        .collect();
    answer.model_digest = Some(result.model_digest.clone());
    answer.result = Some(result);
    if canonical::bytes(&answer).map_err(internal)?.len() + 1 > MAX_STDOUT_BYTES {
        return Err(resource(
            "retained value packet exceeds 64 KiB; reduce selected definitions or compilation scope",
        ));
    }
    Ok(answer)
}

type Projection = (ValueModel, Vec<ValueSource>, Vec<ValueOccurrence>);

/// Private loader: only the verified session-generation path calls it publicly.
fn load_projection(
    store: &CasStore,
    ready: &ReadyGeneration,
    owner: &str,
) -> Result<Option<Projection>, ClewError> {
    if ready.transformed_source.is_some() {
        // Keep transformed-source execution distinct until independently
        // qualified for this operation; never fall back to original bytes.
        return Err(ClewError::new(
            ErrorCode::ProjectModelChanged,
            "transformed Java values are not qualified for this contour",
        ));
    }
    let derived: DerivedAnalysisInputManifest = read(store, &ready.derived_input_manifest)?;
    derived.verify(store)?;
    if derived.repository_snapshot != ready.repository_snapshot {
        return Err(corrupt("Java values have foreign snapshot ownership"));
    }
    let providers: Vec<_> = derived
        .provider_models
        .iter()
        .filter(|p| p.handshake.provider_id == "project-native-java")
        .collect();
    let [provider] = providers.as_slice() else {
        return Err(corrupt("Java values require one exact native provider"));
    };
    if provider.build_model.model.object_schema != JAVA_MODEL_SCHEMA {
        return Err(corrupt("Java value configuration schema is invalid"));
    }
    let model: JavaProjectModel = read(store, &provider.build_model.model)?;
    crate::java_project_model::verify_model(&model)?;
    let descriptors: Vec<_> = provider
        .build_model
        .compilations
        .iter()
        .filter(|d| d.language_uri.as_str() == "language:java")
        .collect();
    let [descriptor] = descriptors.as_slice() else {
        return Err(corrupt(
            "Java values require one exact compilation descriptor",
        ));
    };
    if model.compilation != ready.compilation
        || provider.handshake.provider_digest != model.model_digest
        || descriptor
            .source_roots
            .iter()
            .any(|s| s.tree != ready.repository_snapshot)
    {
        return Err(corrupt(
            "Java value model has foreign compilation or source ownership",
        ));
    }
    if descriptor.canonical_options != provider.build_model.model {
        let options: serde_json::Value = read(store, &descriptor.canonical_options)?;
        let bound: CasObject = serde_json::from_value(
            options
                .get("model")
                .cloned()
                .ok_or_else(|| corrupt("Java value options omit their model"))?,
        )
        .map_err(|_| corrupt("Java value options model is invalid"))?;
        if bound != provider.build_model.model {
            return Err(corrupt(
                "Java value options have foreign configuration ownership",
            ));
        }
    }
    let receipt: IncrementalReceipt = read(store, &ready.incremental_receipt)?;
    let expected_key =
        CompilerStoreKey::create("java-compiler-1", java_adapter_digest()?, descriptor)?;
    if expected_key.key != receipt.compiler_store_key {
        return Err(ClewError::new(
            ErrorCode::ProjectModelChanged,
            "Java value extractor is stale",
        ));
    }
    let generation: GenerationManifest = read(store, &ready.generation)?;
    if generation.fact_count > MAX_FACTS
        || generation
            .shards
            .iter()
            .try_fold(0u64, |size, shard| size.checked_add(shard.size))
            .is_none_or(|bytes| bytes > MAX_CLOSURE_BYTES as u64)
    {
        return Err(resource(
            "Java value generation exceeds retained fact budget",
        ));
    }
    if generation.derived_input_manifest != ready.derived_input_manifest
        || receipt.generation_id != generation.generation_id
    {
        return Err(corrupt(
            "Java values have foreign retained generation ownership",
        ));
    }
    let mut facts = Vec::new();
    let mut total_bytes = 0usize;
    generation.visit_facts(store, |fact| {
        if fact.domain_uri.as_str() != JAVA_COMPILER_FACTS_CAPABILITY
            || fact.payload.object_schema != JAVA_FACT_SCHEMA
        {
            return Err(corrupt(
                "Java value generation includes foreign fact authority",
            ));
        }
        total_bytes = total_bytes
            .checked_add(
                usize::try_from(fact.payload.size)
                    .map_err(|_| resource("Java value payload exceeds host size"))?,
            )
            .filter(|bytes| *bytes <= MAX_CLOSURE_BYTES)
            .ok_or_else(|| resource("Java value payload closure exceeds budget"))?;
        let payload: JavaCompilerFact = read(store, &fact.payload)?;
        if fact.fact_key
            != format!(
                "java:{}",
                canonical::hash(&payload)
                    .map_err(internal)?
                    .trim_start_matches("sha256:")
            )
        {
            return Err(corrupt("Java value fact identity differs from its payload"));
        }
        facts.push(payload);
        Ok(())
    })?;
    facts.sort_by_cached_key(|fact| canonical::bytes(fact).expect("serializable Java fact"));
    let index = JavaCompilerIndex {
        schema: JAVA_INDEX_SCHEMA.into(),
        compilation: model.compilation.clone(),
        model,
        analyzer_digest: canonical::hash_bytes(JAVA_ANALYZER_SOURCE.as_bytes()),
        facts,
        provenance: None,
        source_state: None,
    };
    let scope = java_scope_digest(&index)?;
    let [attempt] = generation.attempts.as_slice() else {
        return Err(corrupt(
            "Java values require one exact retained compiler attempt",
        ));
    };
    if attempt.compilation_id != descriptor.compilation_id
        || attempt.capability.as_str() != JAVA_COMPILER_FACTS_CAPABILITY
        || attempt.completion.scope_digest != scope
        || attempt.completion.fact_count != index.facts.len() as u64
    {
        return Err(corrupt(
            "Java value equations differ from the saved compiler attempt",
        ));
    }
    let snapshot: RepositoryInputSnapshot = read(store, &ready.repository_snapshot)?;
    snapshot.verify()?;
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
    let mut sources = Vec::new();
    for path in &index.model.source_files {
        let object = objects
            .get(path.as_str())
            .ok_or_else(|| corrupt("Java value source is absent from retained snapshot"))?;
        total_bytes = total_bytes
            .checked_add(
                usize::try_from(object.size)
                    .map_err(|_| resource("Java value source exceeds host size"))?,
            )
            .filter(|bytes| *bytes <= MAX_CLOSURE_BYTES)
            .ok_or_else(|| resource("Java value source closure exceeds budget"))?;
        let bytes = store.read(object, MAX_CLOSURE_BYTES)?;
        let digest = canonical::hash_bytes(bytes.bytes());
        let members: Vec<_> = index
            .facts
            .iter()
            .filter(|f| {
                matches!(f,
            JavaCompilerFact::SourceFile { file, source_content_digest, resolution, .. }
                if file == path && source_content_digest == &digest
                    && resolution == "SOURCE_MEMBERSHIP_EXACT")
            })
            .collect();
        if members.len() != 1
            || receipt
                .files
                .iter()
                .filter(|f| f.path == *path && f.content_digest == digest)
                .count()
                != 1
        {
            return Err(corrupt(
                "Java value source digest differs from retained compiler inputs",
            ));
        }
        sources.push(ValueSource {
            file: path.clone(),
            content: (*object).clone(),
            content_digest: digest,
        });
    }
    let selected: Vec<_> = index
        .facts
        .iter()
        .filter_map(|fact| match fact {
            JavaCompilerFact::Declaration {
                symbol_identity,
                value_projection,
                ..
            } if symbol_identity == owner => Some(value_projection.as_ref()),
            _ => None,
        })
        .collect();
    let [Some(projection)] = selected.as_slice() else {
        return Ok(None);
    };
    let source = sources
        .iter()
        .find(|source| source.file == projection.nodes[0].file)
        .ok_or_else(|| corrupt("Java value occurrence source is absent"))?;
    let bytes = store.read(&source.content, MAX_CLOSURE_BYTES)?;
    let text = std::str::from_utf8(bytes.bytes())
        .map_err(|_| corrupt("Java value source is not UTF-8"))?;
    for node in &projection.nodes {
        let start = usize::try_from(node.byte_start)
            .map_err(|_| corrupt("Java value occurrence exceeds host size"))?;
        let end = usize::try_from(node.byte_end)
            .map_err(|_| corrupt("Java value occurrence exceeds host size"))?;
        if start >= end
            || end > text.len()
            || !text.is_char_boundary(start)
            || !text.is_char_boundary(end)
            || text[..start].encode_utf16().count() as u64 != node.start
            || text[..end].encode_utf16().count() as u64 != node.end
            || bytes.bytes()[..start]
                .iter()
                .filter(|byte| **byte == b'\n')
                .count() as u64
                + 1
                != node.start_line
            || bytes.bytes()[..end - 1]
                .iter()
                .filter(|byte| **byte == b'\n')
                .count() as u64
                + 1
                != node.end_line
        {
            return Err(corrupt(
                "Java value occurrence does not match retained source coordinates",
            ));
        }
    }
    let model = crate::java_values::project_index(&index, owner)?;
    let occurrences = projection
        .nodes
        .iter()
        .map(|n| ValueOccurrence {
            definition: n.id.clone(),
            file: n.file.clone(),
            byte_start: n.byte_start,
            byte_end: n.byte_end,
            start_line: n.start_line,
            end_line: n.end_line,
        })
        .collect();
    sources.retain(|source| projection.nodes.iter().any(|n| n.file == source.file));
    Ok(Some((model, sources, occurrences)))
}

fn read<T: DeserializeOwned + Serialize>(
    store: &CasStore,
    object: &CasObject,
) -> Result<T, ClewError> {
    if object.size > MAX_CLOSURE_BYTES as u64 {
        return Err(resource("Java value retained object exceeds budget"));
    }
    let lease = store.read(object, MAX_CLOSURE_BYTES)?;
    let value: T = serde_json::from_slice(lease.bytes())
        .map_err(|_| corrupt("Java value retained object is invalid"))?;
    if canonical::bytes(&value).map_err(internal)? != lease.bytes() {
        return Err(corrupt("Java value retained object is not canonical"));
    }
    Ok(value)
}
fn corrupt(message: &str) -> ClewError {
    ClewError::new(ErrorCode::StateCorrupt, message)
}
fn resource(message: &str) -> ClewError {
    ClewError::new(ErrorCode::ResourceLimit, message)
}
fn internal(error: impl std::fmt::Display) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter_v2::{
        AnalysisAttemptComplete, BuildModel, COMPILATION_SCHEMA, CapabilityUri,
        CompilationDescriptor, DescriptorCompleteness, DescriptorOrigin, FactRecord, LanguageUri,
        PROVIDER_PROTOCOL, ProviderHandshake, ProviderModel, SourceRootDescriptor,
    };
    use crate::generation_service::{
        AnalysisExecutionAuthority, IncrementalExecutionEvidence, IncrementalExecutionMode,
        READY_GENERATION_SCHEMA,
    };
    use crate::generation_v2::{AttemptAuthority, FactRunWriter, finalize_generation};
    use crate::incremental_v2::{
        FileReceipt, FullAnalysisReason, INCREMENTAL_RECEIPT_SCHEMA, IncrementalPlan,
    };
    use crate::java_project_model::JavaBuildSystem;
    use crate::worker::WorkerRequestCounters;
    use serde_json::json;
    use std::{fs, process::Command};

    const OWNER: &str = "method:class:Values#literal()Ljava/lang/String;";
    const SOURCE: &str = "class Values { String literal() { return null; } }\n";

    // Component-binding fixtures, not a claimed compiler run. The managed CLI
    // fixture independently executes javac and enters through a saved session.
    fn fixture(
        change: impl FnOnce(&mut JavaCompilerIndex),
    ) -> (tempfile::TempDir, CasStore, ReadyGeneration) {
        let temporary = tempfile::tempdir().unwrap();
        let repo = temporary.path().join("repo");
        fs::create_dir(&repo).unwrap();
        fs::write(repo.join("Values.java"), SOURCE).unwrap();
        for args in [
            vec!["init", "-q", "-b", "main"],
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
        let state = StateAuthority::open(temporary.path().join("state")).unwrap();
        let store = CasStore::open(&state).unwrap();
        let (_, snapshot) = crate::repository_snapshot::capture(&repo, &store).unwrap();
        let mut model = JavaProjectModel {
            schema: JAVA_MODEL_SCHEMA.into(),
            model_digest: String::new(),
            build_system: JavaBuildSystem::Gradle,
            compilation: ":/main".into(),
            source_files: vec!["Values.java".into()],
            classpath: vec![],
            dependency_sources: vec![],
            release: 17,
            compiler_version: "javac 17".into(),
            compiler_options: vec![],
            annotation_processors: vec![],
            annotation_processor_paths: vec![],
            boundaries: vec![],
        };
        model.model_digest = canonical::hash(&model).unwrap();
        let start = SOURCE.find("null").unwrap();
        let end = start + 4;
        let method_start = SOURCE.find("String literal").unwrap();
        let method_end = SOURCE.find('}').unwrap() + 1;
        let id = format!("v{start}_{end}");
        let fact: JavaCompilerFact = serde_json::from_value(json!({
            "kind":"DECLARATION","schema":JAVA_FACT_SCHEMA,"declarationKind":"METHOD",
            "name":"literal","symbolIdentity":OWNER,"ownerIdentity":"class:Values",
            "modifiers":[],"annotations":[],"file":"Values.java","start":method_start,"end":method_end,
            "byteStart":method_start,"byteEnd":method_end,"startLine":1,"endLine":1,"resolution":"COMPILER_EXACT",
            "valueProjection":{"schema":crate::java_values::SCHEMA,"owner":OWNER,
                "nodes":[
                    {"id":"result","operation":{"kind":"COPY","value":id},"guards":[],"file":"Values.java",
                     "start":method_start,"end":method_end,"byteStart":method_start,"byteEnd":method_end,"startLine":1,"endLine":1},
                    {"id":id,"operation":{"kind":"NULL"},"guards":[],"file":"Values.java",
                     "start":start,"end":end,"byteStart":start,"byteEnd":end,"startLine":1,"endLine":1}],
                "globalBoundaries":["JAVA_VALUE_EFFECT_EXCEPTION_MODEL_UNAVAILABLE"]}
        })).unwrap();
        let source = JavaCompilerFact::SourceFile {
            schema: JAVA_FACT_SCHEMA.into(),
            file: "Values.java".into(),
            source_content_digest: canonical::hash_bytes(SOURCE.as_bytes()),
            resolution: "SOURCE_MEMBERSHIP_EXACT".into(),
        };
        let mut index = JavaCompilerIndex {
            schema: JAVA_INDEX_SCHEMA.into(),
            compilation: ":/main".into(),
            model,
            analyzer_digest: canonical::hash_bytes(JAVA_ANALYZER_SOURCE.as_bytes()),
            facts: vec![fact, source],
            provenance: None,
            source_state: None,
        };
        change(&mut index);
        index
            .facts
            .sort_by_cached_key(|f| canonical::bytes(f).unwrap());
        let scope = java_scope_digest(&index).unwrap();
        let complete = crate::java_adapter_v2::java_completeness(&index, &scope).unwrap();
        let model_object = store
            .put(JAVA_MODEL_SCHEMA, &canonical::bytes(&index.model).unwrap())
            .unwrap();
        let descriptor = CompilationDescriptor {
            schema: COMPILATION_SCHEMA.into(),
            compilation_id: "main".into(),
            language_uri: LanguageUri::parse("language:java").unwrap(),
            source_roots: vec![SourceRootDescriptor {
                logical_name: "project".into(),
                tree: snapshot.clone(),
            }],
            generated_source_roots: vec![],
            classpath: vec![],
            plugins: vec![],
            toolchain: store.put("test/toolchain", b"javac17").unwrap(),
            canonical_options: model_object.clone(),
            dependency_compilation_ids: vec![],
            operations: vec![],
            origin: DescriptorOrigin::ProjectNative,
            completeness: DescriptorCompleteness::Complete,
        };
        let provider = ProviderModel {
            handshake: ProviderHandshake {
                protocol: PROVIDER_PROTOCOL.into(),
                provider_id: "project-native-java".into(),
                provider_digest: index.model.model_digest.clone(),
                build_system_uris: vec!["build:gradle".into()],
            },
            build_model: BuildModel {
                provider_id: "project-native-java".into(),
                model: model_object,
                compilations: vec![descriptor.clone()],
            },
        };
        let (_, derived) =
            DerivedAnalysisInputManifest::create(&store, snapshot.clone(), vec![provider]).unwrap();
        let mut records: Vec<_> = index
            .facts
            .iter()
            .map(|f| {
                let payload = store
                    .put(JAVA_FACT_SCHEMA, &canonical::bytes(f).unwrap())
                    .unwrap();
                FactRecord {
                    fact_key: format!(
                        "java:{}",
                        canonical::hash(f).unwrap().trim_start_matches("sha256:")
                    ),
                    domain_uri: CapabilityUri::parse(JAVA_COMPILER_FACTS_CAPABILITY).unwrap(),
                    payload,
                }
            })
            .collect();
        records.sort_by(|a, b| a.fact_key.cmp(&b.fact_key));
        let mut writer = FactRunWriter::create(&state).unwrap();
        for record in records {
            writer.push(&record).unwrap();
        }
        let attempt = AttemptAuthority {
            compilation_id: "main".into(),
            capability: CapabilityUri::parse(JAVA_COMPILER_FACTS_CAPABILITY).unwrap(),
            completion: AnalysisAttemptComplete {
                scope_digest: scope,
                completeness_receipt: store.put("test/receipt", b"complete").unwrap(),
                fact_count: index.facts.len() as u64,
            },
        };
        let (generation, generation_object) = finalize_generation(
            &store,
            derived.clone(),
            vec![attempt],
            vec![writer.finish().unwrap()],
        )
        .unwrap();
        let receipt = IncrementalReceipt {
            schema: INCREMENTAL_RECEIPT_SCHEMA.into(),
            compiler_store_key: CompilerStoreKey::create(
                "java-compiler-1",
                java_adapter_digest().unwrap(),
                &descriptor,
            )
            .unwrap()
            .key,
            generation_id: generation.generation_id,
            completeness: complete.clone(),
            boundaries: vec![],
            files: vec![FileReceipt {
                path: "Values.java".into(),
                content_digest: canonical::hash_bytes(SOURCE.as_bytes()),
                exported_surface_digest: canonical::hash(&index.facts).unwrap(),
                dependencies: vec![],
            }],
        };
        let ready = ReadyGeneration {
            schema: READY_GENERATION_SCHEMA.into(),
            generation_key: canonical::hash_bytes(b"fixture"),
            runtime_key: canonical::hash_bytes(b"runtime"),
            base_revision: "1".repeat(40),
            compilation: ":/main".into(),
            compiler_version: "javac 17".into(),
            completeness: complete,
            coverage: "COMPLETE".into(),
            certainty: "VERIFIED".into(),
            obligations: vec![],
            repository_snapshot: snapshot,
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
                analysis_execution_authority: AnalysisExecutionAuthority::CompilerProcess,
                subset_analysis_supported: false,
                compiler_output_receipt: None,
                worker_requests: WorkerRequestCounters {
                    open_project_requests: 0,
                    index_files_requests: 0,
                },
            },
        };
        (temporary, store, ready)
    }

    #[test]
    fn retained_binding_is_exact_repeatable_and_independent_of_live_checkout() {
        let (temporary, store, ready) = fixture(|_| {});
        let first = load_projection(&store, &ready, OWNER).unwrap().unwrap();
        fs::write(
            temporary.path().join("repo/Values.java"),
            "changed live file",
        )
        .unwrap();
        let second = load_projection(&store, &ready, OWNER).unwrap().unwrap();
        assert_eq!(first, second);
        let result = crate::thread_values::analyze(
            &first.0,
            &ValueQuery {
                target: "result".into(),
                max_nodes: 128,
                max_transfers: 4096,
            },
        )
        .unwrap();
        assert_eq!(
            result
                .observations
                .iter()
                .find(|v| v.id == "result")
                .unwrap()
                .normal_value_nullness,
            crate::thread_values::Nullness::Null
        );
        assert!(!result.program_semantic_closure);
        assert!(
            load_projection(
                &store,
                &ready,
                "method:class:Foreign#literal()Ljava/lang/String;"
            )
            .unwrap()
            .is_none()
        );
    }

    #[test]
    fn changed_source_digests_and_source_coordinates_are_rejected() {
        for coordinate in [false, true] {
            let (_temporary, store, ready) = fixture(|index| {
                for fact in &mut index.facts {
                    if coordinate {
                        if let JavaCompilerFact::Declaration {
                            value_projection: Some(p),
                            ..
                        } = fact
                        {
                            p.nodes[1].byte_start += 1;
                        }
                    } else if let JavaCompilerFact::SourceFile {
                        source_content_digest,
                        ..
                    } = fact
                    {
                        *source_content_digest = canonical::hash_bytes(b"other source");
                    }
                }
            });
            assert_eq!(
                load_projection(&store, &ready, OWNER).unwrap_err().code,
                ErrorCode::StateCorrupt
            );
        }
    }

    #[test]
    fn foreign_generation_configuration_missing_closure_and_stale_extractor_fail_closed() {
        let (_temporary, store, ready) = fixture(|_| {});
        let mut foreign = ready.clone();
        foreign.repository_snapshot = store
            .put(crate::repository_snapshot::SNAPSHOT_SCHEMA, b"foreign")
            .unwrap();
        assert_eq!(
            load_projection(&store, &foreign, OWNER).unwrap_err().code,
            ErrorCode::StateCorrupt
        );
        let mut missing = ready.clone();
        missing.generation.digest = canonical::hash_bytes(b"unretained");
        assert!(load_projection(&store, &missing, OWNER).is_err());
        let mut stale = ready.clone();
        let mut receipt: IncrementalReceipt = read(&store, &ready.incremental_receipt).unwrap();
        receipt.compiler_store_key = canonical::hash_bytes(b"another configuration or extractor");
        stale.incremental_receipt = store
            .put(
                INCREMENTAL_RECEIPT_SCHEMA,
                &canonical::bytes(&receipt).unwrap(),
            )
            .unwrap();
        assert_eq!(
            load_projection(&store, &stale, OWNER).unwrap_err().code,
            ErrorCode::ProjectModelChanged
        );
        let mut changed = ready.clone();
        let mut derived: DerivedAnalysisInputManifest =
            read(&store, &ready.derived_input_manifest).unwrap();
        derived.provider_models[0].build_model.compilations[0].canonical_options = store
            .put(
                "test/options",
                &canonical::bytes(&json!({"model": ready.repository_snapshot})).unwrap(),
            )
            .unwrap();
        let (_, object) = DerivedAnalysisInputManifest::create(
            &store,
            ready.repository_snapshot.clone(),
            derived.provider_models,
        )
        .unwrap();
        changed.derived_input_manifest = object;
        assert_eq!(
            load_projection(&store, &changed, OWNER).unwrap_err().code,
            ErrorCode::StateCorrupt
        );
    }

    #[test]
    fn old_projection_is_explicitly_unavailable_and_cannot_be_upgraded() {
        let (_temporary, store, ready) = fixture(|index| {
            for fact in &mut index.facts {
                if let JavaCompilerFact::Declaration {
                    value_projection, ..
                } = fact
                {
                    *value_projection = None;
                }
            }
        });
        assert!(load_projection(&store, &ready, OWNER).unwrap().is_none());
    }
}
