//! File reuse must produce the same current evidence as uncached extraction.
use super::{
    model::{Service, ServiceEvidence},
    store::Repository,
    syntax, syntax_file_cache,
};
use crate::{cas::CasStore, error::ErrorCode, state::StateAuthority};
use serde_json::json;
use std::{cell::Cell, fs, path::Path, process::Command};

struct Fixture {
    source: tempfile::TempDir,
    _state: tempfile::TempDir,
    _documentation: tempfile::TempDir,
    cas: CasStore,
    documentation: Repository,
}

impl Fixture {
    fn new() -> Self {
        let source = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        let documentation = tempfile::tempdir().unwrap();
        let cas =
            CasStore::open(&StateAuthority::open(state.path().join("state")).unwrap()).unwrap();
        Repository::init(documentation.path(), "Syntax reuse fixture").unwrap();
        let repo = Repository::open(documentation.path()).unwrap();
        git(source.path(), &["init", "-q"]);
        Self {
            source,
            _state: state,
            _documentation: documentation,
            cas,
            documentation: repo,
        }
    }

    fn write(&self, path: &str, text: &str) {
        let path = self.source.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn commit(&self) -> String {
        git(self.source.path(), &["add", "-A"]);
        git(
            self.source.path(),
            &["commit", "-qm", "Synthetic source change"],
        );
        git(self.source.path(), &["rev-parse", "HEAD"])
    }

    fn cached(&self, service: &Service, revision: &str) -> ServiceEvidence {
        syntax::capture_with_store_cached(
            service,
            self.source.path(),
            revision,
            &self.cas,
            &self.documentation,
        )
        .unwrap()
    }

    fn uncached(&self, service: &Service, revision: &str) -> ServiceEvidence {
        syntax::capture_with_store(service, self.source.path(), revision, &self.cas).unwrap()
    }
}

fn git(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(repo)
        .args(args)
        .env("GIT_AUTHOR_NAME", "Syntax fixture")
        .env("GIT_AUTHOR_EMAIL", "syntax-fixture@example.invalid")
        .env("GIT_COMMITTER_NAME", "Syntax fixture")
        .env("GIT_COMMITTER_EMAIL", "syntax-fixture@example.invalid")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().into()
}

fn service(language: &str) -> Service {
    serde_json::from_value(json!({
        "schema":"codeclew-documentation-service/1.0", "id":"orders", "title":"Orders",
        "repositoryId":"orders", "repository":"https://example.invalid/orders",
        "language":language, "profile":"source-syntax", "targetRef":"HEAD",
        "source":{"roots":["."],"dialect":"declared"}
    }))
    .unwrap()
}

#[test]
fn changed_file_parses_once_while_hits_rebind_all_current_source_evidence() {
    let fixture = Fixture::new();
    let service = service("python");
    fixture.write("kept.py", "def kept():\n    return 1\n");
    fixture.write("changed.py", "def changed():\n    return 2\n");
    let first_revision = fixture.commit();
    syntax::take_file_cache_stats();
    let first = fixture.cached(&service, &first_revision);
    let first_stats = syntax::take_file_cache_stats();
    assert_eq!(
        (first_stats.hits, first_stats.misses, first_stats.parses),
        (0, 2, 2)
    );
    assert_eq!(first, fixture.uncached(&service, &first_revision));

    fixture.write("changed.py", "def changed():\n    return 3\n");
    let revision = fixture.commit();
    let fresh = fixture.uncached(&service, &revision);
    syntax::take_file_cache_stats();
    let reused = fixture.cached(&service, &revision);
    let stats = syntax::take_file_cache_stats();
    assert_eq!((stats.hits, stats.misses, stats.parses), (1, 1, 1));
    assert_eq!(reused, fresh);
    assert_eq!(reused.revision, revision);
    assert_eq!(reused.boundaries, first.boundaries);
    assert_eq!(reused.sources.len(), first.sources.len());
    let old = first
        .sources
        .values()
        .find(|source| source.file == "kept.py")
        .unwrap();
    let current = &reused.sources[&old.id];
    assert_eq!(current.text, old.text);
    assert_ne!(
        current.occurrence.as_ref().unwrap().snapshot,
        old.occurrence.as_ref().unwrap().snapshot
    );
    assert_ne!(current.evidence_digest, old.evidence_digest);
    for source in reused.sources.values() {
        assert_eq!(source.revision, revision);
        assert!(
            source
                .url
                .as_deref()
                .unwrap()
                .contains(&format!("/blob/{revision}/"))
        );
        let content = fs::read_to_string(fixture.source.path().join(&source.file)).unwrap();
        let occurrence = source.occurrence.as_ref().unwrap();
        assert_eq!(
            source.text,
            content[occurrence.start_byte..occurrence.end_byte]
        );
    }
}

#[test]
fn duplicate_bytes_at_another_path_get_distinct_sources_and_facts() {
    let fixture = Fixture::new();
    let service = service("python");
    let text = "def result():\n    return 7\n";
    fixture.write("first.py", text);
    let revision = fixture.commit();
    fixture.cached(&service, &revision);
    fixture.write("other.py", text);
    let revision = fixture.commit();
    let fresh = fixture.uncached(&service, &revision);
    syntax::take_file_cache_stats();
    let reused = fixture.cached(&service, &revision);
    let stats = syntax::take_file_cache_stats();
    assert_eq!((stats.hits, stats.misses, stats.parses), (1, 1, 1));
    assert_eq!(reused, fresh);
    let first = reused
        .sources
        .values()
        .find(|source| source.file == "first.py" && source.text.starts_with("def result():"))
        .unwrap();
    let first_occurrence = first.occurrence.as_ref().unwrap();
    let other = reused
        .sources
        .values()
        .find(|source| {
            source.file == "other.py"
                && source.occurrence.as_ref().is_some_and(|occurrence| {
                    occurrence.start_byte == first_occurrence.start_byte
                        && occurrence.end_byte == first_occurrence.end_byte
                })
        })
        .unwrap();
    assert_eq!(first.text, other.text);
    assert_ne!(first.id, other.id);
    assert_ne!(first.url, other.url);
    for source in [first, other] {
        assert_eq!(source.revision, revision);
        assert!(
            source
                .url
                .as_deref()
                .unwrap()
                .contains(&format!("/blob/{revision}/{}", source.file))
        );
    }
    assert!(
        reused
            .observations
            .values()
            .any(|fact| fact.source_ids.contains(&first.id))
    );
    assert!(
        reused
            .observations
            .values()
            .any(|fact| fact.source_ids.contains(&other.id))
    );
}

#[test]
fn dialect_changes_miss_while_link_changes_rebind_without_parsing() {
    let fixture = Fixture::new();
    let mut service = service("python");
    fixture.write("orders.py", "def result():\n    return 7\n");
    let revision = fixture.commit();
    fixture.cached(&service, &revision);
    service.source.as_mut().unwrap().dialect = "different-declared-dialect".into();
    let fresh = fixture.uncached(&service, &revision);
    syntax::take_file_cache_stats();
    assert_eq!(fixture.cached(&service, &revision), fresh);
    let stats = syntax::take_file_cache_stats();
    assert_eq!((stats.hits, stats.misses, stats.parses), (0, 1, 1));
    service.repository = "https://example.invalid/current-orders".into();
    service.source_link_template = Some("{repository}/commit/{revision}/{file}".into());
    let fresh = fixture.uncached(&service, &revision);
    syntax::take_file_cache_stats();
    let rebound = fixture.cached(&service, &revision);
    let stats = syntax::take_file_cache_stats();
    assert_eq!((stats.hits, stats.misses, stats.parses), (1, 0, 0));
    assert_eq!(rebound, fresh);
    assert!(rebound.sources.values().all(|source| {
        source
            .url
            .as_deref()
            .unwrap()
            .starts_with("https://example.invalid/current-orders/commit/")
    }));
}

#[test]
fn changed_scope_membership_and_annotations_match_uncached_capture() {
    let fixture = Fixture::new();
    let service = service("java");
    fixture.write(
        "Keep.java",
        "@Deprecated class Keep { @Deprecated String result() { return \"ok\"; } }\n",
    );
    fixture.write(
        "Removed.java",
        "class Removed { int result() { return 1; } }\n",
    );
    let revision = fixture.commit();
    fixture.cached(&service, &revision);
    fs::remove_file(fixture.source.path().join("Removed.java")).unwrap();
    fixture.write(
        "Added.java",
        "@Deprecated class Added { int result() { return 2; } }\n",
    );
    let revision = fixture.commit();
    let fresh = fixture.uncached(&service, &revision);
    syntax::take_file_cache_stats();
    let reused = fixture.cached(&service, &revision);
    let stats = syntax::take_file_cache_stats();
    assert_eq!((stats.hits, stats.misses, stats.parses), (1, 1, 1));
    assert_eq!(reused, fresh);
    assert!(
        !reused
            .sources
            .values()
            .any(|source| source.file == "Removed.java")
    );
    assert!(
        reused
            .sources
            .values()
            .any(|source| source.file == "Added.java")
    );
    assert!(reused.observations.values().any(|fact| {
        fact.kind == "SYNTAX_DETAIL"
            && fact.normalized["text"]
                .as_str()
                .is_some_and(|text| text.contains("@Deprecated"))
    }));
}

#[test]
fn partial_parse_is_reparsed_and_never_promoted_to_a_cache_hit() {
    let fixture = Fixture::new();
    let service = service("python");
    let text = "def good():\n    return 1\n\ndef broken(\n";
    fixture.write("orders.py", text);
    let revision = fixture.commit();
    let fresh = fixture.uncached(&service, &revision);
    assert_eq!(fresh.coverage, "PARTIAL");
    assert!(
        fresh
            .boundaries
            .iter()
            .any(|boundary| boundary == "PARSE_ERROR:orders.py")
    );
    for _ in 0..2 {
        syntax::take_file_cache_stats();
        assert_eq!(fixture.cached(&service, &revision), fresh);
        let stats = syntax::take_file_cache_stats();
        assert_eq!((stats.hits, stats.misses, stats.parses), (0, 1, 1));
        assert_eq!(stats.parse_errors, 1);
    }
    let key = syntax_file_cache::producer_key(&service, "orders.py", text).unwrap();
    assert!(
        !fixture
            .documentation
            .path(&syntax_file_cache::manifest_path(&key))
            .unwrap()
            .exists()
    );
}

#[test]
fn complete_producer_identity_and_each_file_domain_part_admit_separate_cache_keys() {
    let fixture = Fixture::new();
    let service = service("python");
    let text = "def result():\n    return 7\n";
    fixture.write("orders.py", text);
    let revision = fixture.commit();
    fixture.cached(&service, &revision);
    let producer = syntax_file_cache::producer_identity();
    let key = syntax_file_cache::producer_key(&service, "orders.py", text).unwrap();
    assert_eq!(
        key,
        syntax_file_cache::key_with_producer(&service, "orders.py", text, producer).unwrap()
    );
    // The production key consumes the complete bundled producer digest. Any
    // replacement parser, grammar or mapping digest uses another admission.
    let other_producer = syntax_file_cache::key_with_producer(
        &service,
        "orders.py",
        text,
        "changed-parser-grammar-normalization-producer",
    )
    .unwrap();
    assert_ne!(key, other_producer);
    assert!(
        syntax_file_cache::load(
            &fixture.documentation,
            &other_producer,
            &service.id,
            "orders.py"
        )
        .unwrap()
        .is_none()
    );
    let mut other_service = service.clone();
    other_service.id = "other-service".into();
    let mut other_language = service.clone();
    other_language.language = "java".into();
    for different in [
        syntax_file_cache::producer_key(&other_service, "orders.py", text).unwrap(),
        syntax_file_cache::producer_key(&other_language, "orders.py", text).unwrap(),
        syntax_file_cache::producer_key(&service, "other.py", text).unwrap(),
        syntax_file_cache::producer_key(&service, "orders.py", &format!("{text}# changed bytes\n"))
            .unwrap(),
    ] {
        assert_ne!(key, different);
        assert!(
            !fixture
                .documentation
                .path(&syntax_file_cache::manifest_path(&different))
                .unwrap()
                .exists()
        );
    }
}

#[test]
fn cache_hits_recharge_overlapping_source_bytes_and_current_source_and_fact_counts() {
    const SOURCE_BUDGET: usize = 32 * 1024 * 1024;
    const FACT_BUDGET: usize = 32_768;
    let fixture = Fixture::new();
    let service = service("python");
    let text = "class Orders:\n    def result(self):\n        return 7\n";
    fixture.write("orders.py", text);
    let revision = fixture.commit();
    let captured = fixture.cached(&service, &revision);
    let key = syntax_file_cache::producer_key(&service, "orders.py", text).unwrap();
    let payload = syntax_file_cache::load(&fixture.documentation, &key, &service.id, "orders.py")
        .unwrap()
        .unwrap();
    let emitted_bytes: usize = payload
        .sources
        .iter()
        .map(|recipe| recipe.end - recipe.start)
        .sum();
    assert!(
        emitted_bytes > text.len(),
        "nested declarations must emit overlapping source ranges"
    );
    let mut empty = captured.clone();
    empty.sources.clear();
    empty.observations.clear();
    empty.entrypoints.clear();
    empty.boundaries.clear();
    let mut at_limit = empty.clone();
    let consumed = Cell::new(SOURCE_BUDGET - emitted_bytes);
    syntax::replay_file_for_test(
        &service,
        "orders.py",
        text,
        &consumed,
        &payload,
        &mut at_limit,
    )
    .unwrap();
    assert_eq!(consumed.get(), SOURCE_BUDGET);
    assert_eq!(at_limit.sources.len(), captured.sources.len());
    assert_eq!(at_limit.observations.len(), payload.observations.len());
    let consumed = Cell::new(SOURCE_BUDGET - emitted_bytes + 1);
    let error = syntax::replay_file_for_test(
        &service,
        "orders.py",
        text,
        &consumed,
        &payload,
        &mut empty.clone(),
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::SliceBudgetExceeded);

    let mut source_full = empty.clone();
    let source = captured.sources.values().next().unwrap();
    for index in 0..FACT_BUDGET {
        let mut occupied = source.clone();
        occupied.id = format!("occupied-source-{index}");
        source_full.sources.insert(occupied.id.clone(), occupied);
    }
    let error = syntax::replay_file_for_test(
        &service,
        "orders.py",
        text,
        &Cell::new(0),
        &payload,
        &mut source_full,
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::SliceBudgetExceeded);

    let mut facts_full = empty;
    let observation = payload.observations.values().next().unwrap();
    for index in 0..FACT_BUDGET {
        let mut occupied = observation.clone();
        occupied.id = format!("occupied-fact-{index}");
        facts_full
            .observations
            .insert(occupied.id.clone(), occupied);
    }
    let error = syntax::replay_file_for_test(
        &service,
        "orders.py",
        text,
        &Cell::new(0),
        &payload,
        &mut facts_full,
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::SliceBudgetExceeded);
}

#[test]
fn corrupt_envelope_or_payload_is_refused_without_silent_reparse() {
    let fixture = Fixture::new();
    let service = service("python");
    let text = "def result():\n    return 7\n";
    fixture.write("orders.py", text);
    let revision = fixture.commit();
    fixture.cached(&service, &revision);
    let key = syntax_file_cache::producer_key(&service, "orders.py", text).unwrap();
    let manifest = fixture
        .documentation
        .path(&syntax_file_cache::manifest_path(&key))
        .unwrap();
    let mut payload =
        syntax_file_cache::load(&fixture.documentation, &key, &service.id, "orders.py")
            .unwrap()
            .unwrap();
    fs::write(&manifest, b"malformed cache envelope").unwrap();
    syntax::take_file_cache_stats();
    let error = syntax::capture_with_store_cached(
        &service,
        fixture.source.path(),
        &revision,
        &fixture.cas,
        &fixture.documentation,
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::StateCorrupt);
    assert_eq!(syntax::take_file_cache_stats().parses, 0);

    // Persist valid object bytes with an invalid bound path. Object integrity
    // alone must not admit another occurrence under this file's manifest key.
    payload.path = "another-file.py".into();
    syntax_file_cache::save(&fixture.documentation, &payload).unwrap();
    syntax::take_file_cache_stats();
    let error = syntax::capture_with_store_cached(
        &service,
        fixture.source.path(),
        &revision,
        &fixture.cas,
        &fixture.documentation,
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::StateCorrupt);
    assert_eq!(syntax::take_file_cache_stats().parses, 0);

    let mut envelope: serde_json::Value = super::store::read(&manifest, 4096).unwrap();
    envelope["payload"]["digest"] = json!(format!("sha256:{}", "0".repeat(64)));
    fs::write(&manifest, serde_json::to_vec(&envelope).unwrap()).unwrap();
    syntax::take_file_cache_stats();
    let error = syntax::capture_with_store_cached(
        &service,
        fixture.source.path(),
        &revision,
        &fixture.cas,
        &fixture.documentation,
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::StateCorrupt);
    assert_eq!(syntax::take_file_cache_stats().parses, 0);
}

#[test]
fn extraction_budget_errors_do_not_create_a_reusable_payload() {
    let fixture = Fixture::new();
    let service = service("python");
    let text = format!("value = {}1{}\n", "(".repeat(300), ")".repeat(300));
    fixture.write("orders.py", &text);
    let revision = fixture.commit();
    let fresh_error =
        syntax::capture_with_store(&service, fixture.source.path(), &revision, &fixture.cas)
            .unwrap_err();
    assert_eq!(fresh_error.code, ErrorCode::SliceBudgetExceeded);
    for _ in 0..2 {
        syntax::take_file_cache_stats();
        let error = syntax::capture_with_store_cached(
            &service,
            fixture.source.path(),
            &revision,
            &fixture.cas,
            &fixture.documentation,
        )
        .unwrap_err();
        assert_eq!(error.code, fresh_error.code);
        let stats = syntax::take_file_cache_stats();
        assert_eq!((stats.hits, stats.misses, stats.parses), (0, 1, 1));
    }
    let key = syntax_file_cache::producer_key(&service, "orders.py", &text).unwrap();
    assert!(
        !fixture
            .documentation
            .path(&syntax_file_cache::manifest_path(&key))
            .unwrap()
            .exists()
    );
}
