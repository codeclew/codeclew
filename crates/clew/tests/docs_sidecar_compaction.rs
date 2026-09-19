//! Legacy sidecar maintenance must preserve immutable evidence and fail closed.
#![cfg(unix)]
#[path = "support/documentation.rs"]
mod support;

use clew::documentation::{cache, check::Check, store::Repository, work};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::{MetadataExt, symlink},
    path::{Path, PathBuf},
};
use support::Fixture;

fn legacy(f: &Fixture, payload: &[u8]) -> PathBuf {
    let repo = Repository::open(&f.docs).unwrap();
    let reference = cache::put(&repo, "fixture/payload", payload).unwrap();
    let directory = f.docs.join(cache::OBJECT_ROOT).join(&reference.digest);
    fs::write(
        directory.join("meta.json"),
        serde_json::to_vec(&json!({"schema":"legacy/advisory", "size":payload.len()})).unwrap(),
    )
    .unwrap();
    directory
}

fn plan(f: &Fixture, name: &str) -> (PathBuf, Value) {
    let output = f.temp.path().join(name);
    let result = f.ok(&[
        "docs",
        "cache",
        "compact-sidecars",
        "--plan-output",
        output.to_str().unwrap(),
    ]);
    (output, result)
}

fn apply(f: &Fixture, path: &Path, limit: usize, cursor: Option<&str>) -> (i32, Value) {
    let limit = limit.to_string();
    let mut args = vec![
        "docs",
        "cache",
        "compact-sidecars",
        "--apply",
        "--plan",
        path.to_str().unwrap(),
        "--limit",
        &limit,
        "--max-bytes",
        "33554432",
    ];
    if let Some(cursor) = cursor {
        args.extend(["--cursor", cursor]);
    }
    f.run(&args)
}

fn payloads(f: &Fixture) -> BTreeMap<String, Vec<u8>> {
    fs::read_dir(f.docs.join(cache::OBJECT_ROOT))
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            (
                entry.file_name().to_string_lossy().into_owned(),
                fs::read(entry.path().join("object.json")).unwrap(),
            )
        })
        .collect()
}

#[test]
fn compacted_sidecars_preserve_saved_snapshot_work_and_reader_without_sources() {
    let f = Fixture::new();
    let source = f.service("orders");
    let repo = Repository::open(&f.docs).unwrap();
    let snapshot = f.checked().save_snapshot(&repo).unwrap();
    f.ok(&["docs", "render", "--snapshot", &snapshot]);
    let request = f.input("request.json", &json!({"schema":"codeclew-documentation-work-request/1.0", "audience":"Maintainers", "entrypoint":"section-entities"}));
    let prepared = f.ok(&[
        "docs",
        "work",
        "prepare",
        "--subject",
        "service:orders",
        "--snapshot",
        &snapshot,
        "--input",
        request.to_str().unwrap(),
    ]);
    let id = prepared["work"].as_str().unwrap();
    let before_work = serde_json::to_value(work::load(&repo, id).unwrap()).unwrap();
    let before_check =
        serde_json::to_value(Check::load_snapshot(&repo, &snapshot).unwrap()).unwrap();
    let before = payloads(&f);
    for (digest, bytes) in &before {
        fs::write(
            f.docs
                .join(cache::OBJECT_ROOT)
                .join(digest)
                .join("meta.json"),
            serde_json::to_vec(&json!({"schema":"legacy/advisory","size":bytes.len()})).unwrap(),
        )
        .unwrap();
    }
    let index = fs::read(f.docs.join("docs/index.html")).unwrap();
    fs::rename(&source, source.with_extension("offline")).unwrap();
    let latest = f.docs.join(".codeclew/cache/latest-check.json");
    fs::write(&latest, b"Never reacquire during maintenance").unwrap();
    let context = f.ok(&[
        "docs",
        "context",
        "--service",
        "orders",
        "--snapshot",
        &snapshot,
    ]);
    let (path, _) = plan(&f, "sidecars.jsonl");
    assert!(
        before.keys().all(|d| f
            .docs
            .join(cache::OBJECT_ROOT)
            .join(d)
            .join("meta.json")
            .exists()),
        "planning must not delete"
    );
    let mut cursor = None;
    let mut removed = 0;
    loop {
        let (code, report) = apply(&f, &path, 3, cursor.as_deref());
        assert_eq!(code, 0, "{report}");
        assert!(report["visited"].as_u64().unwrap() <= 3);
        removed += report["removed"].as_u64().unwrap();
        cursor = report["nextCursor"].as_str().map(str::to_owned);
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(removed as usize, before.len());
    assert_eq!(payloads(&f), before);
    assert_eq!(
        serde_json::to_value(Check::load_snapshot(&repo, &snapshot).unwrap()).unwrap(),
        before_check
    );
    assert_eq!(
        serde_json::to_value(work::load(&repo, id).unwrap()).unwrap(),
        before_work
    );
    assert_eq!(
        f.ok(&[
            "docs",
            "context",
            "--service",
            "orders",
            "--snapshot",
            &snapshot
        ]),
        context
    );
    assert_eq!(fs::read(f.docs.join("docs/index.html")).unwrap(), index);
    assert_eq!(
        fs::read(latest).unwrap(),
        b"Never reacquire during maintenance"
    );
    let (_, repeated) = apply(&f, &path, before.len() + 1, None);
    assert_eq!(repeated["removed"], 0);
    assert_eq!(payloads(&f), before);
}

#[test]
fn incomplete_foreign_and_corrupt_cursor_plans_cannot_mutate_sidecars() {
    let f = Fixture::new();
    let dir = legacy(&f, b"retained evidence");
    let original = fs::read(dir.join("meta.json")).unwrap();
    let (path, _) = plan(&f, "plan.jsonl");
    let data = fs::read_to_string(&path).unwrap();
    let mut lines: Vec<_> = data.lines().collect();
    lines.pop();
    let incomplete = f.temp.path().join("incomplete.jsonl");
    fs::write(&incomplete, lines.join("\n") + "\n").unwrap();
    assert_ne!(apply(&f, &incomplete, 10, None).0, 0);
    assert_eq!(fs::read(dir.join("meta.json")).unwrap(), original);
    assert_ne!(apply(&f, &path, 10, Some("corrupt-cursor")).0, 0);
    let other = Fixture::new();
    let other_dir = legacy(&other, b"retained evidence");
    assert_ne!(apply(&other, &path, 10, None).0, 0);
    assert_eq!(fs::read(other_dir.join("meta.json")).unwrap(), original);
    assert_eq!(fs::read(dir.join("meta.json")).unwrap(), original);
}

#[test]
fn changed_payload_metadata_and_directory_symlink_are_preserved() {
    let f = Fixture::new();
    let changed = legacy(&f, b"same size old");
    let swapped = legacy(&f, b"outside untouched");
    let metadata = legacy(&f, b"metadata replacement");
    let (path, _) = plan(&f, "plan.jsonl");
    fs::write(changed.join("object.json"), b"same size new").unwrap();
    let replacement = b"{\"schema\":\"unknown\",\"size\":20,\"keep\":true}";
    fs::write(metadata.join("meta.json"), replacement).unwrap();
    let outside = f.temp.path().join("outside");
    fs::rename(&swapped, &outside).unwrap();
    symlink(&outside, &swapped).unwrap();
    let outside_meta = fs::read(outside.join("meta.json")).unwrap();
    let (_, report) = apply(&f, &path, 10, None);
    assert_eq!(report["removed"], 0, "{report}");
    assert!(changed.join("meta.json").is_file());
    assert_eq!(
        fs::read(changed.join("object.json")).unwrap(),
        b"same size new"
    );
    assert_eq!(fs::read(metadata.join("meta.json")).unwrap(), replacement);
    assert_eq!(fs::read(outside.join("meta.json")).unwrap(), outside_meta);
    assert_eq!(
        fs::read(outside.join("object.json")).unwrap(),
        b"outside untouched"
    );
}

#[test]
fn malformed_unknown_and_linked_metadata_are_not_candidates() {
    let f = Fixture::new();
    let valid = legacy(&f, b"valid");
    let malformed = legacy(&f, b"malformed");
    let unknown = legacy(&f, b"unknown");
    let linked = legacy(&f, b"linked");
    fs::write(malformed.join("meta.json"), b"{broken").unwrap();
    fs::write(
        unknown.join("meta.json"),
        br#"{"schema":"future","size":7,"newField":true}"#,
    )
    .unwrap();
    let outside = f.temp.path().join("outside-meta");
    fs::write(&outside, b"preserve external metadata").unwrap();
    fs::remove_file(linked.join("meta.json")).unwrap();
    symlink(&outside, linked.join("meta.json")).unwrap();
    let (path, _) = plan(&f, "plan.jsonl");
    let (code, report) = apply(&f, &path, 10, None);
    assert_eq!(code, 0, "{report}");
    assert_eq!(report["removed"], 1);
    assert!(!valid.join("meta.json").exists());
    assert!(malformed.join("meta.json").exists());
    assert!(unknown.join("meta.json").exists());
    assert_eq!(fs::read(outside).unwrap(), b"preserve external metadata");
}

#[test]
fn planning_and_cursor_pages_scale_past_legacy_enumeration_limits() {
    let f = Fixture::new();
    let count = 9001usize;
    let fixture_started = std::time::Instant::now();
    let mut sidecar_allocated = 0u64;
    for n in 0..count {
        let path = legacy(&f, format!("immutable evidence item {n}").as_bytes());
        sidecar_allocated += fs::metadata(path.join("meta.json")).unwrap().blocks() * 512;
    }
    let before = payloads(&f);
    let fixture_seconds = fixture_started.elapsed().as_secs_f64();
    let plan_started = std::time::Instant::now();
    let (path, _) = plan(&f, "large-plan.jsonl");
    let plan_seconds = plan_started.elapsed().as_secs_f64();
    let plan_bytes = fs::metadata(&path).unwrap().len();
    let mut cursor = None;
    let mut removed = 0u64;
    let mut read_bytes = 0u64;
    let mut calls = 0;
    let apply_started = std::time::Instant::now();
    let mut max_page_seconds = 0.0_f64;
    loop {
        let page_started = std::time::Instant::now();
        let (code, report) = apply(&f, &path, 1000, cursor.as_deref());
        max_page_seconds = max_page_seconds.max(page_started.elapsed().as_secs_f64());
        assert_eq!(code, 0, "{report}");
        removed += report["removed"].as_u64().unwrap();
        read_bytes += report["planBytesRead"].as_u64().unwrap();
        calls += 1;
        cursor = report["nextCursor"].as_str().map(str::to_owned);
        if cursor.is_none() {
            break;
        }
        assert!(calls <= 11, "cursor did not advance");
    }
    let apply_seconds = apply_started.elapsed().as_secs_f64();
    assert_eq!(removed as usize, count);
    assert_eq!(payloads(&f), before);
    assert!(
        read_bytes < plan_bytes * 3,
        "continuation appears to rescan the plan: {read_bytes}/{plan_bytes}"
    );
    assert!(
        sidecar_allocated > fs::metadata(&path).unwrap().blocks() * 512,
        "plan allocation erased gross storage saving"
    );
    eprintln!(
        "sidecar fixture: {count} removed; old sidecar allocation {sidecar_allocated}, plan bytes {plan_bytes}, plan bytes read {read_bytes}, calls {calls}, fixture seconds {fixture_seconds:.3}, plan seconds {plan_seconds:.3}, apply seconds {apply_seconds:.3}, max page seconds {max_page_seconds:.3}"
    );
}

#[test]
fn same_bytes_replacement_is_preserved_and_new_objects_do_not_invalidate_cursor() {
    let f = Fixture::new();
    let changed = legacy(&f, b"identical metadata replacement");
    let _others: Vec<_> = (0..3)
        .map(|n| legacy(&f, format!("original {n}").as_bytes()))
        .collect();
    let (path, _) = plan(&f, "plan.jsonl");
    let original = fs::read(changed.join("meta.json")).unwrap();
    // Holding the original inode makes this a deterministic replacement test.
    let held = fs::File::open(changed.join("meta.json")).unwrap();
    let replacement = changed.join("replacement");
    fs::write(&replacement, &original).unwrap();
    fs::rename(&replacement, changed.join("meta.json")).unwrap();
    assert_ne!(
        held.metadata().unwrap().ino(),
        fs::metadata(changed.join("meta.json")).unwrap().ino()
    );
    let (code, first) = apply(&f, &path, 1, None);
    assert_eq!(code, 0, "{first}");
    let new = legacy(&f, b"added after first page");
    let mut cursor = first["nextCursor"].as_str().map(str::to_owned);
    let mut removed = first["removed"].as_u64().unwrap();
    let mut calls = 1;
    while let Some(next) = cursor {
        let (code, report) = apply(&f, &path, 1, Some(&next));
        assert_eq!(code, 0, "{report}");
        removed += report["removed"].as_u64().unwrap();
        cursor = report["nextCursor"].as_str().map(str::to_owned);
        calls += 1;
        assert!(calls <= 5);
    }
    assert_eq!(removed, 3);
    assert_eq!(fs::read(changed.join("meta.json")).unwrap(), original);
    assert!(new.join("meta.json").is_file());
}

#[test]
fn sidecar_plan_cannot_be_written_inside_objects_through_an_alias() {
    let f = Fixture::new();
    let directory = legacy(&f, b"preserve all objects");
    let alias = f.temp.path().join("object-alias");
    symlink(f.docs.join(cache::OBJECT_ROOT), &alias).unwrap();
    let output = alias.join("untrusted-plan.jsonl");
    let (code, _) = f.run(&[
        "docs",
        "cache",
        "compact-sidecars",
        "--plan-output",
        output.to_str().unwrap(),
    ]);
    assert_ne!(code, 0);
    assert!(!output.exists());
    assert!(directory.join("meta.json").is_file());
    assert_eq!(
        fs::read(directory.join("object.json")).unwrap(),
        b"preserve all objects"
    );
}

#[test]
fn checksum_valid_plan_cannot_supply_arbitrary_mutation_paths() {
    let f = Fixture::new();
    let dir = legacy(&f, b"retained target");
    let original = fs::read(dir.join("meta.json")).unwrap();
    let (path, _) = plan(&f, "plan.jsonl");
    let data = fs::read_to_string(&path).unwrap();
    let lines: Vec<_> = data.lines().collect();
    assert_eq!(lines.len(), 3);
    for (n, relative) in [
        "../../outside/meta.json",
        "/tmp/meta.json",
        "objects/not-a-digest/meta.json",
    ]
    .iter()
    .enumerate()
    {
        let mut entry: Value = serde_json::from_str(lines[1]).unwrap();
        entry["relative"] = json!(relative);
        let records = format!("{}\n{}\n", lines[0], serde_json::to_string(&entry).unwrap());
        let mut footer: Value = serde_json::from_str(lines[2]).unwrap();
        footer["recordsChecksum"] = json!(clew::canonical::hash_bytes(records.as_bytes()));
        let forged = f.temp.path().join(format!("forged-{n}.jsonl"));
        fs::write(
            &forged,
            format!("{records}{}\n", serde_json::to_string(&footer).unwrap()),
        )
        .unwrap();
        assert_ne!(
            apply(&f, &forged, 10, None).0,
            0,
            "accepted path {relative}"
        );
        assert_eq!(fs::read(dir.join("meta.json")).unwrap(), original);
    }
}

#[test]
fn byte_budget_preserves_next_candidate_and_net_accounting_includes_control_files() {
    fn usage(path: &Path) -> (u64, u64) {
        let m = fs::symlink_metadata(path).unwrap();
        let mut result = (m.len(), m.blocks() * 512);
        if m.is_dir() {
            for item in fs::read_dir(path).unwrap() {
                let child = usage(&item.unwrap().path());
                result.0 += child.0;
                result.1 += child.1;
            }
        }
        result
    }
    let f = Fixture::new();
    for n in 0..101 {
        legacy(&f, format!("net accounting item {n}").as_bytes());
    }
    let cache_root = f.docs.join(".codeclew/cache");
    let before_usage = usage(&cache_root);
    let before_payloads = payloads(&f);
    let (path, _) = plan(&f, "accounted-plan.jsonl");
    let (code, blocked) = f.run(&[
        "docs",
        "cache",
        "compact-sidecars",
        "--apply",
        "--plan",
        path.to_str().unwrap(),
        "--limit",
        "10",
        "--max-bytes",
        "1",
    ]);
    assert_eq!(code, 0, "{blocked}");
    assert_eq!(blocked["visited"], 0);
    assert_eq!(blocked["removed"], 0);
    assert!(blocked["requiredBytes"].as_u64().unwrap() > 1);
    let cursor = blocked["nextCursor"].as_str().unwrap();
    let (code, done) = apply(&f, &path, 200, Some(cursor));
    assert_eq!(code, 0, "{done}");
    assert_eq!(done["removed"], 101);
    assert!(done["nextCursor"].is_null());
    assert_eq!(payloads(&f), before_payloads);
    let after_cache = usage(&cache_root);
    let plan_usage = usage(&path);
    let maintenance_usage = usage(&cache_root.join("sidecar-compaction"));
    let after_allocated = after_cache.1 + plan_usage.1;
    assert!(before_usage.1 > after_allocated);
    eprintln!(
        "sidecar net fixture: objects 101; before cache logical {} allocated {}; after cache logical {} allocated {}; plan logical {} allocated {}; maintenance logical {} allocated {}; net allocated reduction {}; net logical delta {}",
        before_usage.0,
        before_usage.1,
        after_cache.0,
        after_cache.1,
        plan_usage.0,
        plan_usage.1,
        maintenance_usage.0,
        maintenance_usage.1,
        before_usage.1 - after_allocated,
        (after_cache.0 + plan_usage.0) as i64 - before_usage.0 as i64
    );
}
