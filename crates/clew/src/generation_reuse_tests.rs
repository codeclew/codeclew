//! Focused fixtures for exact post-OpenProject analysis reuse.
//!
//! The fixture exercises archive lookup only: an exact, fully validated
//! immutable result is recovered while unrelated archive/head state exists.
//! It does not model a real A -> B -> A compiler sequence or claim that a
//! compiler was run for the historical states.

use super::*;
use crate::adapter_v2::{BuildModel, FactRecord, ProviderHandshake, ProviderModel};
use crate::generation_v2::{AttemptAuthority, FactRunWriter, finalize_generation};
use crate::incremental_v2::{CompilerStoreKey, IncrementalReceipt};
use crate::kotlin_engine::{
    KOTLIN_PROJECT_SEMANTICS_SCHEMA, KotlinProjectSemantics, KotlinSemanticEngine,
};
use crate::repository_snapshot::{RepositoryInputSnapshot, SNAPSHOT_SCHEMA};
use crate::runtime::RuntimeMode;
use crate::session::SESSION_SCHEMA;
use crate::state::StateAuthority;
use std::path::PathBuf;

struct ReuseFixture {
    _root: tempfile::TempDir,
    state: StateAuthority,
    store: CasStore,
    path: PathBuf,
    session: SessionAuthority,
    prepared: PreparedGenerationAuthority,
    compiler_store: CompilerStoreKey,
    ready: ReadyGeneration,
    receipt: IncrementalReceipt,
}

fn digest(c: char) -> String {
    format!("sha256:{}", c.to_string().repeat(64))
}

fn snapshot(store: &CasStore, marker: char) -> (RepositoryInputSnapshot, CasObject) {
    let mut value = RepositoryInputSnapshot {
        schema: SNAPSHOT_SCHEMA.into(),
        snapshot_id: String::new(),
        staged_view_digest: digest(marker),
        cached_view_digest: digest(marker),
        untracked_view_digest: digest(marker),
        index: Vec::new(),
        worktree: Vec::new(),
    };
    value.snapshot_id = canonical::hash(&value).unwrap();
    let object = store
        .put(SNAPSHOT_SCHEMA, &canonical::bytes(&value).unwrap())
        .unwrap();
    (value, object)
}

fn write_ready(fixture: &ReuseFixture, ready: &ReadyGeneration) {
    fixture
        .state
        .write_private_atomic(&fixture.path, &canonical::bytes(ready).unwrap())
        .unwrap();
}

fn expect_state_corrupt<T>(result: Result<T, ClewError>, message_fragment: &str) {
    let error = match result {
        Ok(_) => panic!("authority rejection must return an error"),
        Err(error) => error,
    };
    assert_eq!(error.code, ErrorCode::StateCorrupt);
    assert!(
        error.message.contains(message_fragment),
        "expected {message_fragment:?} in state-corruption message, got {:?}",
        error.message
    );
}

fn fixture() -> ReuseFixture {
    fixture_for_language(false)
}

// Both variants are synthetic immutable-generation fixtures. They exercise
// checkpoint integrity, never a native compiler or analyzer invocation.
fn fixture_for_language(java: bool) -> ReuseFixture {
    let root = tempfile::tempdir().unwrap();
    let state = StateAuthority::open(root.path().join("v2")).unwrap();
    let store = CasStore::open(&state).unwrap();
    let (_snapshot_value, snapshot_object) = snapshot(&store, 'a');
    let runtime_key = digest('b');
    let base_revision = digest('c');
    let compilation = "main".to_owned();

    let descriptor = CompilationDescriptor {
        schema: COMPILATION_SCHEMA.into(),
        compilation_id: compilation.clone(),
        language_uri: LanguageUri::parse(if java {
            "language:java"
        } else {
            "language:kotlin"
        })
        .unwrap(),
        source_roots: vec![SourceRootDescriptor {
            logical_name: "main".into(),
            tree: snapshot_object.clone(),
        }],
        generated_source_roots: Vec::new(),
        classpath: Vec::new(),
        toolchain: store.put("test/toolchain/1", b"toolchain").unwrap(),
        plugins: Vec::new(),
        canonical_options: store.put("test/options/1", b"options").unwrap(),
        dependency_compilation_ids: Vec::new(),
        operations: Vec::new(),
        origin: DescriptorOrigin::ProjectNative,
        completeness: DescriptorCompleteness::Complete,
    };
    let provider = ProviderModel {
        handshake: ProviderHandshake {
            protocol: PROVIDER_PROTOCOL.into(),
            provider_id: "fixture-provider".into(),
            provider_digest: runtime_key.clone(),
            build_system_uris: vec!["build:fixture".into()],
        },
        build_model: BuildModel {
            provider_id: "fixture-provider".into(),
            model: store.put("test/model/1", b"fixture-model").unwrap(),
            compilations: vec![descriptor.clone()],
        },
    };
    let (_, derived_object) =
        DerivedAnalysisInputManifest::create(&store, snapshot_object.clone(), vec![provider])
            .unwrap();
    let project_semantics = KotlinProjectSemantics {
        schema: KOTLIN_PROJECT_SEMANTICS_SCHEMA.into(),
        project_compiler_version: "2.4.10".into(),
        compiler_version_authority: "fixture".into(),
        language_version: None,
        api_version: None,
        jvm_target: None,
        compiler_plugins: Vec::new(),
        unstable_compiler_options: Vec::new(),
    };
    let prepared = PreparedGenerationAuthority {
        schema: PREPARED_AUTHORITY_SCHEMA.into(),
        runtime_key: runtime_key.clone(),
        repository_snapshot: snapshot_object.clone(),
        compilation: compilation.clone(),
        project_semantics,
        semantic_engine: KotlinSemanticEngine::Kotlin24.authority(),
        adapter_digest: digest('d'),
        descriptor: descriptor.clone(),
        derived_input_manifest: derived_object.clone(),
    };
    let compiler_store = CompilerStoreKey::create(
        if java {
            "java-compiler-1"
        } else {
            KOTLIN_ADAPTER_CONTRACT_ID
        },
        digest('d'),
        &descriptor,
    )
    .unwrap();

    let payload = store
        .put(
            "test/fact/1",
            &canonical::bytes(&serde_json::json!({"name":"Widget","file":if java { "Main.java" } else { "main.kt" }})).unwrap(),
        )
        .unwrap();
    let fact = FactRecord {
        fact_key: "declaration:Widget".into(),
        domain_uri: CapabilityUri::parse("analysis:fixture").unwrap(),
        payload,
    };
    let mut run_writer = FactRunWriter::create(&state).unwrap();
    run_writer.push(&fact).unwrap();
    let run = run_writer.finish().unwrap();
    let completeness = CompletenessVector::verified_complete(digest('f')).unwrap();
    let completeness_receipt = store.put("test/completeness/1", b"complete").unwrap();
    let completion = AnalysisAttemptComplete {
        scope_digest: digest('f'),
        completeness_receipt,
        fact_count: 1,
    };
    let (generation, generation_object) = finalize_generation(
        &store,
        derived_object.clone(),
        vec![AttemptAuthority {
            compilation_id: compilation.clone(),
            capability: CapabilityUri::parse("analysis:fixture").unwrap(),
            completion,
        }],
        vec![run],
    )
    .unwrap();
    let (_, query_index_object) =
        build_query_index(&store, &generation, generation_object.clone()).unwrap();
    let receipt = IncrementalReceipt {
        schema: INCREMENTAL_RECEIPT_SCHEMA.into(),
        compiler_store_key: compiler_store.key.clone(),
        generation_id: generation.generation_id.clone(),
        files: Vec::new(),
        boundaries: Vec::new(),
        completeness: completeness.clone(),
    };
    receipt.validate().unwrap();
    let receipt_object = store
        .put(
            INCREMENTAL_RECEIPT_SCHEMA,
            &canonical::bytes(&receipt).unwrap(),
        )
        .unwrap();
    let ready = ReadyGeneration {
        schema: READY_GENERATION_SCHEMA.into(),
        generation_key: final_generation_key(
            &runtime_key,
            &base_revision,
            &snapshot_object,
            &compilation,
            &derived_object,
            false,
        )
        .unwrap(),
        runtime_key: runtime_key.clone(),
        base_revision: base_revision.clone(),
        compilation: compilation.clone(),
        compiler_version: if java { "javac 21" } else { "2.4.10" }.into(),
        completeness: completeness.clone(),
        coverage: coverage_label(&completeness).into(),
        certainty: certainty_label(&completeness).into(),
        obligations: obligation_codes(&completeness),
        incremental: IncrementalExecutionEvidence {
            schema: INCREMENTAL_EVIDENCE_SCHEMA.into(),
            planned: IncrementalPlan::Full {
                reason: FullAnalysisReason::NoParent,
            },
            executed: IncrementalExecutionMode::Full,
            analysis_execution_authority: if java {
                AnalysisExecutionAuthority::CompilerProcess
            } else {
                AnalysisExecutionAuthority::CompilerWorker
            },
            subset_analysis_supported: false,
            worker_requests: WorkerRequestCounters {
                open_project_requests: u64::from(!java),
                index_files_requests: u64::from(!java),
            },
        },
        incremental_receipt: receipt_object,
        repository_snapshot: snapshot_object,
        derived_input_manifest: derived_object,
        generation: generation_object,
        query_index: query_index_object,
        transformed_source: None,
    };
    let session = SessionAuthority {
        schema: SESSION_SCHEMA.into(),
        authority_digest: digest('1'),
        session_id: "session:reuse-fixture".into(),
        repository_key: digest('2'),
        base_revision,
        target_ref: "refs/heads/main".into(),
        target_oid: digest('3'),
        runtime_key,
        runtime_mode: RuntimeMode::Release,
        language: if java {
            SessionLanguage::Java
        } else {
            SessionLanguage::Kotlin
        },
        compilations: vec![compilation],
        generation_jobs: Some(1),
        model_cache_policy: ModelCachePolicy::NonCacheable,
        model_cache_authority: None,
        maven_settings_digest: None,
        profile: None,
        working_tree: None,
        created_unix_ms: 0,
    };
    let path = state
        .session_root(&session.session_id)
        .unwrap()
        .join("generation.json");
    ReuseFixture {
        _root: root,
        state,
        store,
        path,
        session,
        prepared,
        compiler_store,
        ready,
        receipt,
    }
}

#[test]
fn exact_analysis_absent_binding_returns_none() {
    let fixture = fixture();
    assert!(
        load_exact_analysis(
            &fixture.state,
            &fixture.store,
            &fixture.path,
            &fixture.session,
            &fixture.ready.compilation,
            &fixture.ready.generation_key,
            &fixture.prepared,
            &fixture.compiler_store,
        )
        .unwrap()
        .is_none()
    );
}

#[test]
fn exact_saved_analysis_reuses_after_unrelated_archive_state() {
    let fixture = fixture();
    write_ready(&fixture, &fixture.ready);
    let archive = fixture
        .path
        .parent()
        .unwrap()
        .join("archive")
        .join("b.json");
    fixture
        .state
        .write_private_atomic(&archive, b"unrelated archive state")
        .unwrap();
    let found = load_exact_analysis(
        &fixture.state,
        &fixture.store,
        &fixture.path,
        &fixture.session,
        &fixture.ready.compilation,
        &fixture.ready.generation_key,
        &fixture.prepared,
        &fixture.compiler_store,
    )
    .unwrap()
    .expect("exact A binding survives unrelated archive state");
    assert_eq!(found.0, fixture.ready);
    assert_eq!(found.1, fixture.receipt);
}

#[test]
fn expected_generation_snapshot_and_compiler_authority_mismatches_refuse_reuse() {
    let fixture = fixture();
    write_ready(&fixture, &fixture.ready);
    let mut mismatched_prepared = fixture.prepared.clone();
    mismatched_prepared.repository_snapshot.digest = digest('9');
    let mut mismatched_derived = fixture.prepared.clone();
    mismatched_derived.derived_input_manifest.digest = digest('a');
    let mut mismatched_engine = fixture.prepared.clone();
    mismatched_engine.semantic_engine.analyzer_compiler_version = "2.3.0".into();
    let mut mismatched_store = fixture.compiler_store.clone();
    mismatched_store.key = digest('8');
    for (generation_key, prepared, compiler_store) in [
        (
            digest('7'),
            fixture.prepared.clone(),
            fixture.compiler_store.clone(),
        ),
        (
            fixture.ready.generation_key.clone(),
            mismatched_prepared,
            fixture.compiler_store.clone(),
        ),
        (
            fixture.ready.generation_key.clone(),
            mismatched_derived,
            fixture.compiler_store.clone(),
        ),
        (
            fixture.ready.generation_key.clone(),
            mismatched_engine,
            fixture.compiler_store.clone(),
        ),
        (
            fixture.ready.generation_key.clone(),
            fixture.prepared.clone(),
            mismatched_store,
        ),
    ] {
        let result = load_exact_analysis(
            &fixture.state,
            &fixture.store,
            &fixture.path,
            &fixture.session,
            &fixture.ready.compilation,
            &generation_key,
            &prepared,
            &compiler_store,
        );
        expect_state_corrupt(
            result,
            "saved analysis differs from current exact OpenProject authority",
        );
    }

    let mut mismatched_runtime = fixture.session.clone();
    mismatched_runtime.runtime_key = digest('7');
    expect_state_corrupt(
        load_exact_analysis(
            &fixture.state,
            &fixture.store,
            &fixture.path,
            &mismatched_runtime,
            &fixture.ready.compilation,
            &fixture.ready.generation_key,
            &fixture.prepared,
            &fixture.compiler_store,
        ),
        "ready generation session authority is invalid",
    );

    let mut mismatched_session = fixture.session.clone();
    mismatched_session.base_revision = digest('6');
    expect_state_corrupt(
        load_exact_analysis(
            &fixture.state,
            &fixture.store,
            &fixture.path,
            &mismatched_session,
            &fixture.ready.compilation,
            &fixture.ready.generation_key,
            &fixture.prepared,
            &fixture.compiler_store,
        ),
        "ready generation session authority is invalid",
    );
}

#[test]
fn missing_or_corrupt_descendant_refuses_reuse() {
    let fixture = fixture();
    let mut missing_generation = fixture.ready.clone();
    missing_generation.generation = CasObject {
        schema: crate::cas::CAS_OBJECT_SCHEMA.into(),
        object_schema: GENERATION_SCHEMA.into(),
        digest: digest('6'),
        size: 1,
    };
    write_ready(&fixture, &missing_generation);
    expect_state_corrupt(
        load_exact_analysis(
            &fixture.state,
            &fixture.store,
            &fixture.path,
            &fixture.session,
            &fixture.ready.compilation,
            &fixture.ready.generation_key,
            &fixture.prepared,
            &fixture.compiler_store,
        ),
        "CAS object is missing or unsafe",
    );

    let mut corrupt_query = fixture.ready.clone();
    corrupt_query.query_index = fixture
        .store
        .put(QUERY_INDEX_SCHEMA, b"not canonical query")
        .unwrap();
    write_ready(&fixture, &corrupt_query);
    expect_state_corrupt(
        load_exact_analysis(
            &fixture.state,
            &fixture.store,
            &fixture.path,
            &fixture.session,
            &fixture.ready.compilation,
            &fixture.ready.generation_key,
            &fixture.prepared,
            &fixture.compiler_store,
        ),
        "query index binding is invalid",
    );
}

#[test]
fn reuse_preserves_completeness_receipt_and_query_binding() {
    let fixture = fixture();
    write_ready(&fixture, &fixture.ready);
    let (ready, receipt) = load_exact_analysis(
        &fixture.state,
        &fixture.store,
        &fixture.path,
        &fixture.session,
        &fixture.ready.compilation,
        &fixture.ready.generation_key,
        &fixture.prepared,
        &fixture.compiler_store,
    )
    .unwrap()
    .unwrap();
    assert_eq!(ready.completeness, fixture.ready.completeness);
    assert_eq!(receipt, fixture.receipt);
    let query = load_query_index(&fixture.store, &ready).unwrap();
    assert_eq!(query.generation, ready.generation);
    assert_eq!(receipt.compiler_store_key, fixture.compiler_store.key);
}

#[test]
fn reuse_preserves_non_complete_completeness_without_upgrade() {
    let fixture = fixture();
    let incomplete = CompletenessVector {
        schema: COMPLETENESS_VECTOR_SCHEMA.into(),
        support: Support::Supported,
        coverage: Coverage::Unknown,
        certainty: Certainty::Unsure {
            check_set: vec!["VERIFY_SCOPE".into()],
        },
        obligations: vec![VerificationObligation {
            code: "VERIFY_SCOPE".into(),
            subject: vec![digest('f')],
            publication_blocking: true,
        }],
    };
    incomplete.validate().unwrap();
    assert!(!incomplete.publishable());

    let mut ready = fixture.ready.clone();
    ready.completeness = incomplete.clone();
    ready.coverage = coverage_label(&incomplete).into();
    ready.certainty = certainty_label(&incomplete).into();
    ready.obligations = obligation_codes(&incomplete);
    let mut receipt = fixture.receipt.clone();
    receipt.completeness = incomplete.clone();
    receipt.validate().unwrap();
    ready.incremental_receipt = fixture
        .store
        .put(
            INCREMENTAL_RECEIPT_SCHEMA,
            &canonical::bytes(&receipt).unwrap(),
        )
        .unwrap();
    write_ready(&fixture, &ready);

    let (reused, reused_receipt) = load_exact_analysis(
        &fixture.state,
        &fixture.store,
        &fixture.path,
        &fixture.session,
        &ready.compilation,
        &ready.generation_key,
        &fixture.prepared,
        &fixture.compiler_store,
    )
    .unwrap()
    .unwrap();
    assert_eq!(reused.completeness, incomplete);
    assert!(!reused.completeness.publishable());
    assert_eq!(reused_receipt.completeness, incomplete);
}

fn history_path(fixture: &ReuseFixture) -> PathBuf {
    let repository_root = fixture.state.root().join("repos").join("fixture-history");
    fixture
        .state
        .directory_at(&repository_root.join("generations/analysis"))
        .unwrap();
    analysis_history_path(&repository_root, &fixture.prepared, &fixture.compiler_store).unwrap()
}

fn publish_history(fixture: &ReuseFixture) -> PathBuf {
    let path = history_path(fixture);
    publish_incremental_head(
        &fixture.state,
        &fixture.store,
        &path,
        &fixture.ready,
        &fixture.compiler_store.key,
    )
    .unwrap();
    path
}

#[test]
fn content_analysis_reuses_complete_history_across_base_revisions() {
    let fixture = fixture();
    let path = publish_history(&fixture);
    assert!(
        !fixture.path.exists(),
        "history-only publication must not depend on a session generation binding"
    );
    assert!(
        !fixture.path.parent().unwrap().join("archive").exists(),
        "history-only publication must not depend on a revision archive"
    );
    let mut current_session = fixture.session.clone();
    current_session.base_revision = digest('9');

    let found = load_content_analysis(
        &fixture.state,
        &fixture.store,
        &path,
        &current_session,
        &fixture.prepared,
        &fixture.compiler_store,
    )
    .unwrap()
    .expect("history entry should be reusable for a new revision");
    assert_eq!(found.0, fixture.ready);
    assert_eq!(found.1, fixture.receipt);
    assert_eq!(
        load_query_index(&fixture.store, &found.0)
            .unwrap()
            .generation,
        found.0.generation
    );
    assert_eq!(found.1.completeness, fixture.ready.completeness);
}

#[test]
fn content_history_key_changes_for_authority_drift_and_old_entry_is_rejected() {
    let fixture = fixture();
    let path = publish_history(&fixture);
    let repository_root = fixture.state.root().join("repos").join("fixture-history");

    let cases = [
        {
            let mut prepared = fixture.prepared.clone();
            prepared.runtime_key = digest('9');
            ("runtime", prepared, fixture.compiler_store.clone(), {
                let mut session = fixture.session.clone();
                session.runtime_key = digest('9');
                session
            })
        },
        {
            let mut prepared = fixture.prepared.clone();
            prepared.repository_snapshot.digest = digest('9');
            (
                "snapshot",
                prepared,
                fixture.compiler_store.clone(),
                fixture.session.clone(),
            )
        },
        {
            let mut prepared = fixture.prepared.clone();
            prepared.derived_input_manifest.digest = digest('9');
            (
                "derived",
                prepared,
                fixture.compiler_store.clone(),
                fixture.session.clone(),
            )
        },
        {
            let mut compiler_store = fixture.compiler_store.clone();
            compiler_store.key = digest('9');
            (
                "compiler store",
                fixture.prepared.clone(),
                compiler_store,
                fixture.session.clone(),
            )
        },
        {
            let mut prepared = fixture.prepared.clone();
            prepared.semantic_engine = KotlinSemanticEngine::Kotlin23.authority();
            (
                "engine",
                prepared,
                fixture.compiler_store.clone(),
                fixture.session.clone(),
            )
        },
        {
            let mut prepared = fixture.prepared.clone();
            prepared.compilation = "other".into();
            (
                "compilation",
                prepared,
                fixture.compiler_store.clone(),
                fixture.session.clone(),
            )
        },
    ];

    for (label, prepared, compiler_store, session) in cases {
        let changed_path =
            analysis_history_path(&repository_root, &prepared, &compiler_store).unwrap();
        assert_ne!(
            path, changed_path,
            "{label} must participate in history identity"
        );
        expect_state_corrupt(
            load_content_analysis(
                &fixture.state,
                &fixture.store,
                &path,
                &session,
                &prepared,
                &compiler_store,
            ),
            "saved analysis differs",
        );
    }
}

#[test]
fn content_history_missing_closure_is_a_hard_failure() {
    let fixture = fixture();
    let path = publish_history(&fixture);
    let bytes = fixture
        .state
        .read_private_file(&path, MAX_BINDING_BYTES)
        .unwrap();
    let mut head: IncrementalHead = serde_json::from_slice(&bytes).unwrap();
    head.ready = CasObject {
        schema: crate::cas::CAS_OBJECT_SCHEMA.into(),
        object_schema: READY_GENERATION_SCHEMA.into(),
        digest: digest('9'),
        size: 1,
    };
    fixture
        .state
        .write_private_atomic(&path, &canonical::bytes(&head).unwrap())
        .unwrap();
    expect_state_corrupt(
        load_content_analysis(
            &fixture.state,
            &fixture.store,
            &path,
            &fixture.session,
            &fixture.prepared,
            &fixture.compiler_store,
        ),
        "CAS object is missing or unsafe",
    );
}

#[test]
fn content_history_corrupt_binding_is_a_hard_failure() {
    let fixture = fixture();
    let path = publish_history(&fixture);
    fixture
        .state
        .write_private_atomic(&path, b"corrupt history binding")
        .unwrap();
    expect_state_corrupt(
        load_content_analysis(
            &fixture.state,
            &fixture.store,
            &path,
            &fixture.session,
            &fixture.prepared,
            &fixture.compiler_store,
        ),
        "incremental head binding is invalid",
    );
}

#[test]
fn missing_content_history_returns_none() {
    let fixture = fixture();
    let path = history_path(&fixture);
    assert!(
        load_content_analysis(
            &fixture.state,
            &fixture.store,
            &path,
            &fixture.session,
            &fixture.prepared,
            &fixture.compiler_store,
        )
        .unwrap()
        .is_none()
    );
}

#[test]
fn content_history_is_a_storage_root_for_the_complete_cas_closure() {
    let fixture = fixture();
    let path = publish_history(&fixture);
    let _orphan = fixture.store.put("test/orphan/1", b"orphan").unwrap();

    let report = crate::cas::storage_status(&fixture.state).unwrap();
    assert!(report.reachable_objects > 0);
    assert!(!report.collection_blocked);
    assert_eq!(
        report.reclaimable_loose_objects, 1,
        "the deliberately unrooted object should be the sole reclaimable loose object"
    );
    assert_eq!(report.reclaimable_packs, 0);

    let found = load_content_analysis(
        &fixture.state,
        &fixture.store,
        &path,
        &fixture.session,
        &fixture.prepared,
        &fixture.compiler_store,
    )
    .unwrap();
    assert!(
        found.is_some(),
        "history root must retain its full CAS closure"
    );
}

#[test]
fn engine_startup_hint_is_scoped_and_never_requires_analysis_authority() {
    let fixture = fixture();
    let hint = || {
        cached_engine_hint(
            &fixture.state,
            &fixture.store,
            &fixture.path,
            &fixture.session.runtime_key,
            &fixture.ready.compilation,
        )
    };
    assert_eq!(hint(), None);
    let publish_hint = |ready: &ReadyGeneration| {
        let head = IncrementalHead {
            schema: INCREMENTAL_HEAD_SCHEMA.into(),
            compiler_store_key: fixture.compiler_store.key.clone(),
            receipt: ready.incremental_receipt.clone(),
            ready: fixture
                .store
                .put(READY_GENERATION_SCHEMA, &canonical::bytes(ready).unwrap())
                .unwrap(),
        };
        fixture
            .state
            .write_private_atomic(&fixture.path, &canonical::bytes(&head).unwrap())
            .unwrap();
    };
    publish_hint(&fixture.ready);
    assert_eq!(hint(), Some(KotlinSemanticEngine::Kotlin24));
    for field in ["runtime", "compilation", "compiler", "schema"] {
        let mut ready = fixture.ready.clone();
        match field {
            "runtime" => ready.runtime_key = digest('9'),
            "compilation" => ready.compilation = "other".into(),
            "compiler" => ready.compiler_version = "unknown".into(),
            _ => ready.schema = "unknown".into(),
        }
        publish_hint(&ready);
        assert_eq!(hint(), None, "{field} must not qualify a startup hint");
    }
    // A launch hint does not certify any analysis closure. The normal live
    // OpenProject and exact-analysis validation remain mandatory downstream.
    let mut ready = fixture.ready.clone();
    ready.generation.digest = digest('6');
    publish_hint(&ready);
    assert_eq!(hint(), Some(KotlinSemanticEngine::Kotlin24));
    fixture
        .state
        .write_private_atomic(&fixture.path, b"invalid hint")
        .unwrap();
    assert_eq!(hint(), None);
}

#[test]
fn java_checkpoint_recovers_without_session_publication_and_preserves_receipt() {
    let fixture = fixture_for_language(true);
    let checkpoint = fixture.path.with_file_name("checkpoint.json");
    publish_java_analysis_checkpoint(&fixture.state, &fixture.store, &checkpoint, &fixture.ready)
        .unwrap();
    assert!(!fixture.state.private_file_exists(&fixture.path).unwrap());
    let mut next_session = fixture.session.clone();
    next_session.session_id = "session:next-java-request".into();
    let recovered = load_java_analysis_checkpoint(
        &fixture.state,
        &fixture.store,
        &checkpoint,
        &next_session,
        &fixture.ready.compilation,
        &fixture.ready.generation_key,
        &fixture.ready.derived_input_manifest,
        &fixture.compiler_store,
    )
    .unwrap()
    .unwrap();
    assert_eq!(recovered, fixture.ready);
    write_java_analysis_request(&fixture.state, &fixture.path, &recovered, "ELIGIBLE", true)
        .unwrap();
    let bytes = fixture
        .state
        .read_private_file(
            &fixture.path.with_extension("java-analysis.json"),
            MAX_BINDING_BYTES,
        )
        .unwrap();
    let observation: JavaAnalysisRequest = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(observation.lookup, "HIT");
    assert_eq!(observation.java_analyzer_starts, 0);
    assert_eq!(
        recovered.incremental.executed,
        IncrementalExecutionMode::Full
    );
}

#[test]
fn java_checkpoint_mismatch_and_corruption_are_not_cache_misses() {
    let fixture = fixture_for_language(true);
    let path = &fixture.path;
    let load = |key: &str, input: &CasObject| {
        load_java_analysis_checkpoint(
            &fixture.state,
            &fixture.store,
            path,
            &fixture.session,
            &fixture.ready.compilation,
            key,
            input,
            &fixture.compiler_store,
        )
    };
    assert!(
        load(
            &fixture.ready.generation_key,
            &fixture.ready.derived_input_manifest
        )
        .unwrap()
        .is_none()
    );
    publish_java_analysis_checkpoint(&fixture.state, &fixture.store, path, &fixture.ready).unwrap();
    expect_state_corrupt(
        load(&digest('e'), &fixture.ready.derived_input_manifest),
        "result authority",
    );
    let mut different_input = fixture.ready.derived_input_manifest.clone();
    different_input.digest = digest('e');
    expect_state_corrupt(
        load(&fixture.ready.generation_key, &different_input),
        "input authority",
    );
    fixture
        .state
        .write_private_atomic(path, b"{broken")
        .unwrap();
    expect_state_corrupt(
        load(
            &fixture.ready.generation_key,
            &fixture.ready.derived_input_manifest,
        ),
        "checkpoint is invalid",
    );
}
