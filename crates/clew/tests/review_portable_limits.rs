//! Regression coverage for R5 of the 2026-09-15 commit-and-cache review: the
//! shared portable-cache limit must bound output files that were already
//! written and are being re-verified, not only files captured at write time.
//!
//! Extends the sparse verify_outputs test to a full structured-publication
//! lifecycle: a valid >64 MiB publication survives reload, verification and
//! historical readback; the 128 MiB boundary and one byte above assert the
//! shared typed budget bound; and a digest/edited/missing conflict is a
//! distinct typed outcome from a budget spill, so an oversized publication is
//! rejected without becoming the current generation.

use clew::documentation::bindings::{Bindings, verify_outputs};
use clew::documentation::check::PORTABLE_CACHE_MAX_BYTES;
use clew::documentation::history::{Command as HistoryCommand, Publication};
use clew::documentation::store::Repository;
use clew::error::ErrorCode;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// Streaming sha256 of `size` zero bytes without materializing the buffer,
/// bounding aggregate test memory for multi-megabyte records.
fn digest_of_zeros(size: usize) -> String {
    let chunk = vec![0u8; 1024 * 1024];
    let mut hasher = Sha256::new();
    let mut remaining = size;
    while remaining > 0 {
        let take = remaining.min(chunk.len());
        hasher.update(&chunk[..take]);
        remaining -= take;
    }
    format!("sha256:{}", hex::encode(hasher.finalize()))
}

/// Build a bindings value with a single output hash for `file`→`digest`.
fn bindings_with(file: &str, digest: &str) -> Bindings {
    let mut hashes = Map::new();
    hashes.insert(file.to_string(), Value::String(digest.to_string()));
    serde_json::from_value(json!({
        "schema": "codeclew-documentation-bindings/1.3",
        "inputDigest": "input",
        "renderer": "codeclew-documentation-html/1.13",
        "extractor": "codeclew-documentation-jvm/1.2",
        "revisions": {}, "coverage": {}, "catalogues": {}, "fragments": {},
        "observations": {}, "narratives": {}, "outputHashes": hashes,
        "retainedSources": {}, "sectionStates": {}, "targetRevisions": {},
        "updateFailures": {}, "acceptedVersions": {},
    }))
    .unwrap()
}

/// Stage a real structured publication on disk: an output file of `size`
/// bytes, its bindings manifest, a frozen publication manifest and optionally
/// a current index pointer (first line `<!-- codeclew-bundle {id} -->`).
/// Returns the bindings so callers can verify/reload through the real code.
/// `id` must be 64 lowercase hex digits for history readback.
fn stage_publication(
    repo: &Repository,
    id: &str,
    file: &str,
    size: usize,
    set_index: bool,
) -> Bindings {
    let dir = repo.path(&format!("docs/generated/{id}")).unwrap();
    let path = dir.join(file);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let handle = fs::File::create(&path).unwrap();
    handle.set_len(size as u64).unwrap();
    let digest = digest_of_zeros(size);
    let binding = bindings_with(file, &digest);

    let publication = Publication {
        schema: "codeclew-documentation-publication/1.0".into(),
        id: id.into(),
        parent: None,
        ordinal: 1,
        input_digest: "input".into(),
        target_revisions: Default::default(),
        sections: Default::default(),
        explanation_versions: Default::default(),
        observed_tags: Default::default(),
        evidence_packages: vec![],
        files: BTreeMap::from([(file.to_string(), digest)]),
    };
    fs::write(
        dir.join("bindings.json"),
        serde_json::to_vec(&binding).unwrap(),
    )
    .unwrap();
    fs::write(
        dir.join("publication.json"),
        serde_json::to_vec(&publication).unwrap(),
    )
    .unwrap();
    if set_index {
        // Current-generation pointer so bindings::baseline can resolve the bundle.
        fs::write(
            repo.path("docs/index.html").unwrap(),
            format!("<!-- codeclew-bundle {id} -->\n"),
        )
        .unwrap();
    }
    binding
}

fn repo_at(root: &Path) -> Repository {
    fs::write(
        root.join("codeclew-docs.yaml"),
        "schema: codeclew-documentation/1.0\ntitle: Test\n",
    )
    .unwrap();
    Repository::open(root).unwrap()
}

/// A valid >64 MiB structured publication survives baseline reload, output
/// verification and historical readback through the real code.
#[test]
fn large_structured_publication_survives_reload_verify_and_history() {
    let root = tempfile::tempdir().unwrap();
    let repo = repo_at(root.path());
    let id = "a".repeat(64);
    let file = "chunk.bin";
    // >64 MiB and safely under the shared 128 MiB portable budget.
    let size = (PORTABLE_CACHE_MAX_BYTES as usize) / 2 + 1;

    let binding = stage_publication(&repo, &id, file, size, true);

    // Reload the persisted bindings manifest through the real portable-bound
    // reader and confirm the large output authority survives.
    let manifest_path = repo
        .path(&format!("docs/generated/{id}/bindings.json"))
        .unwrap();
    let loaded: Bindings =
        clew::documentation::store::read(&manifest_path, PORTABLE_CACHE_MAX_BYTES).unwrap();
    assert_eq!(
        loaded.output_hashes.get(file),
        binding.output_hashes.get(file),
        "reload must preserve the large output authority"
    );

    // Verification of the already-written large output passes at this size.
    verify_outputs(&repo, &id, &binding).unwrap();
    verify_outputs(&repo, &id, &loaded).unwrap();

    // Historical readback of the exact version succeeds.
    let value = clew::documentation::history::run(HistoryCommand::Show {
        root: repo.root.clone(),
        id: id.clone(),
        kind: "files".into(),
        cursor: None,
        limit: 20,
    })
    .unwrap();
    assert!(
        serde_json::to_string(&value).unwrap().contains(file),
        "history readback must surface the large publication file: {value}"
    );
}

/// The shared 128 MiB bound is asserted at-limit (accepted) and one byte above
/// (a typed budget spill), and is distinct from a digest/edited/missing
/// conflict which remains a WwConflict tampering error.
#[test]
fn portable_boundary_typed_budget_vs_tampering() {
    let root = tempfile::tempdir().unwrap();
    let repo = repo_at(root.path());
    let limit = PORTABLE_CACHE_MAX_BYTES as usize;

    // At the shared limit an already-written output is accepted and intact.
    let at_limit = stage_publication(&repo, &"b".repeat(64), "at.bin", limit, true);
    verify_outputs(&repo, &"b".repeat(64), &at_limit).unwrap();

    // One byte over the shared limit is a typed budget spill, not a conflict.
    let over = stage_publication(&repo, &"c".repeat(64), "over.bin", limit + 1, true);
    let error = verify_outputs(&repo, &"c".repeat(64), &over).unwrap_err();
    assert_eq!(
        error.code,
        ErrorCode::SliceBudgetExceeded,
        "one-byte-over must assert the typed portable budget bound: {error:?}"
    );

    // A digest mismatch at the same (small) size is a tampering conflict, a
    // distinct typed outcome from the budget spill.
    let tampered_dir = repo
        .path(&format!("docs/generated/{}", "d".repeat(64)))
        .unwrap();
    fs::create_dir_all(&tampered_dir).unwrap();
    fs::write(tampered_dir.join("page.bin"), b"edited").unwrap();
    let tampered = bindings_with("page.bin", &digest_of_zeros(6));
    let error = verify_outputs(&repo, &"d".repeat(64), &tampered).unwrap_err();
    assert_eq!(
        error.code,
        ErrorCode::WwConflict,
        "a digest mismatch must be a tampering conflict, not a budget spill: {error:?}"
    );

    // A missing output is a distinct unavailable outcome (InvalidInput), not
    // a budget spill (SliceBudgetExceeded) and not a content conflict.
    let missing = bindings_with(
        "gone.bin",
        "sha256:0000000000000000000000000000000000000000000000000000000000000000",
    );
    let error = verify_outputs(&repo, &"e".repeat(64), &missing).unwrap_err();
    assert_eq!(
        error.code,
        ErrorCode::InvalidInput,
        "a missing output must be a distinct unavailable outcome: {error:?}"
    );
}

/// An oversized output is rejected by the budget gate and never becomes the
/// current generation: the index still points at the complete prior bundle.
#[test]
fn oversized_publication_fails_without_incomplete_generation() {
    let root = tempfile::tempdir().unwrap();
    let repo = repo_at(root.path());
    let limit = PORTABLE_CACHE_MAX_BYTES as usize;

    // A complete, under-limit publication is the current generation.
    let good_id = "f".repeat(64);
    let good = stage_publication(&repo, &good_id, "page.bin", 1024, true);
    verify_outputs(&repo, &good_id, &good).unwrap();
    let before = fs::read_to_string(repo.path("docs/index.html").unwrap()).unwrap();
    assert!(before.contains(&good_id));

    // An oversized bundle is rejected by the verifier budget gate and does not
    // replace the current generation pointer (index left untouched).
    let over_id = "9".repeat(64);
    let over = stage_publication(&repo, &over_id, "huge.bin", limit + 1, false);
    let error = verify_outputs(&repo, &over_id, &over).unwrap_err();
    assert_eq!(error.code, ErrorCode::SliceBudgetExceeded);
    let after = fs::read_to_string(repo.path("docs/index.html").unwrap()).unwrap();
    assert_eq!(
        after, before,
        "the oversized attempt must not move the current-generation pointer"
    );
    assert!(
        after.contains(&good_id),
        "current generation must remain the complete prior bundle"
    );
}
