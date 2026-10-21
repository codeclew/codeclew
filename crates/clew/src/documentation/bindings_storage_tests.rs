use super::*;
use serde_json::json;

#[path = "bindings_storage_scale_tests.rs"]
mod scale;

fn logical(authority: &str) -> Value {
    let source = |revision: &str, text: &str| {
        json!({"id":"source","service":"orders",
        "revision":revision,"file":"src/Orders.java","startLine":1,"endLine":1,
        "text":text,"textDigest":canonical::hash_bytes(text.as_bytes()),
        "evidenceDigest":"capture","authority":authority})
    };
    let observation = json!({"id":"method","kind":"SYMBOL","service":"orders","symbol":"Orders.run",
        "normalized":{"name":"run"},"digest":digest(&json!({"name":"run"})).unwrap(),"sourceIds":["source"]});
    let content = json!({"text":"The original retained explanation."});
    json!({"schema":"codeclew-documentation-bindings/1.4","inputDigest":"input","renderer":"test/1","extractor":"fixture",
        "revisions":{"orders":"new"},"coverage":{"orders":authority},"catalogues":{},"outputHashes":{},"narratives":{},
        "retainedSources":{"source":source("new","return current();")},"observations":{"method":observation},
        "fragments":{"service:orders/original":{"subject":"service:orders","content":content,"contentDigest":digest(&content).unwrap(),
            "dependencies":{"method":observation["digest"]},"sources":{},"evidence":{"revisions":{"orders":"old"},
                "observations":{"method":observation},"sources":{"source":source("old","return original();")}}}},
        "influenceScopes":{},"sectionStates":{},"targetRevisions":{},"updateFailures":{},"acceptedVersions":{}})
}

#[test]
fn legacy_and_partitioned_semantics_preserve_historical_occurrences() {
    for authority in ["SOURCE_SYNTAX", "COMPILER_EXACT"] {
        for schema in [
            "codeclew-documentation-bindings/1.4",
            "codeclew-documentation-bindings/1.5",
        ] {
            let temp = tempfile::tempdir().unwrap();
            Repository::init(temp.path(), "Portable bindings").unwrap();
            let repo = Repository::open(temp.path()).unwrap();
            let mut original = logical(authority);
            original["schema"] = json!(schema);
            let legacy = super::super::bindings::validate_retained_bindings(
                &repo,
                &bytes(&original).unwrap(),
            )
            .unwrap();
            let encoded = encode(&repo, original.clone(), None, false).unwrap();
            encoded.persist(&repo).unwrap();
            let (resolved, stats) = resolve(&repo, &encoded.manifest).unwrap();
            assert_eq!(resolved, original);
            assert!(stats.objects > 0);
            let current =
                super::super::bindings::validate_retained_bindings(&repo, &encoded.manifest)
                    .unwrap();
            assert_eq!(bytes(&legacy).unwrap(), bytes(&current).unwrap());
            assert_eq!(resolved["retainedSources"]["source"]["revision"], "new");
            assert_eq!(
                resolved["fragments"]["service:orders/original"]["evidence"]["sources"]["source"]["text"],
                "return original();"
            );
            let unchanged = encode(&repo, original, Some(&encoded.manifest), false).unwrap();
            assert_eq!(encoded.manifest, unchanged.manifest);
            assert!(
                unchanged
                    .new_objects
                    .keys()
                    .all(|id| encoded.new_objects.contains_key(id))
            );
        }
    }
}

#[test]
fn corrupted_objects_are_not_repaired_and_reading_projection_is_not_evidence() {
    let temp = tempfile::tempdir().unwrap();
    Repository::init(temp.path(), "Portable bindings").unwrap();
    let repo = Repository::open(temp.path()).unwrap();
    let encoded = encode(&repo, logical("SOURCE_SYNTAX"), None, false).unwrap();
    encoded.persist(&repo).unwrap();
    let object = &encoded.references[0];
    let path = repo.path(&object_path(object).unwrap()).unwrap();
    fs::write(&path, b"corrupt").unwrap();
    assert!(encoded.persist(&repo).is_err());
    assert_eq!(fs::read(path).unwrap(), b"corrupt");
    assert!(resolve(&repo, &encoded.manifest).is_err());
    let fresh = tempfile::tempdir().unwrap();
    Repository::init(fresh.path(), "Reading only").unwrap();
    let fresh = Repository::open(fresh.path()).unwrap();
    let reading = encode(&fresh, logical("SOURCE_SYNTAX"), None, true).unwrap();
    reading.persist(&fresh).unwrap();
    assert!(
        resolve(&fresh, &reading.manifest)
            .unwrap_err()
            .message
            .contains("cannot be admitted")
    );
}

fn repository() -> (tempfile::TempDir, Repository) {
    let temp = tempfile::tempdir().unwrap();
    Repository::init(temp.path(), "Portable bindings").unwrap();
    let repo = Repository::open(temp.path()).unwrap();
    (temp, repo)
}

fn publish(repo: &Repository, mut value: Value, generation: usize) -> String {
    let previous = super::super::bindings::baseline(repo).unwrap();
    let previous_bytes = fs::read(repo.path("docs/index.html").unwrap()).ok();
    let input = repo.input_digest().unwrap();
    value["inputDigest"] = json!(input);
    let id = format!("{generation:064x}");
    super::super::render::commit_bundle(
        repo,
        &id,
        serde_json::from_value(value).unwrap(),
        BTreeMap::new(),
        &format!("<!-- codeclew-bundle {id} -->\n<html><body>Portable publication</body></html>"),
        &input,
        previous.as_ref(),
        previous_bytes.as_deref(),
    )
    .unwrap();
    id
}

#[test]
fn cold_publication_closure_supports_baseline_history_and_export_without_a_database() {
    let (_temp, repo) = repository();
    let id = publish(&repo, logical("SOURCE_SYNTAX"), 1);
    let manifest_path = format!("docs/generated/{id}/bindings.json");
    let raw = fs::read(repo.path(&manifest_path).unwrap()).unwrap();
    let unchanged = publish(&repo, logical("SOURCE_SYNTAX"), 1);
    assert_eq!(unchanged, id);
    assert_eq!(fs::read(repo.path(&manifest_path).unwrap()).unwrap(), raw);
    let copy = tempfile::tempdir().unwrap();
    let mut paths = vec!["codeclew-docs.yaml".to_owned(), "docs/index.html".into()];
    paths.extend(
        fs::read_dir(repo.path(&format!("docs/generated/{id}")).unwrap())
            .unwrap()
            .map(|p| {
                format!(
                    "docs/generated/{id}/{}",
                    p.unwrap().file_name().to_str().unwrap()
                )
            }),
    );
    paths.extend(
        closure(&raw)
            .unwrap()
            .iter()
            .map(|r| object_path(r).unwrap()),
    );
    for path in paths {
        let destination = copy.path().join(&path);
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::copy(repo.path(&path).unwrap(), destination).unwrap();
    }
    let cold = Repository::open_portable(copy.path()).unwrap();
    assert!(cold.portable_read_only);
    assert_eq!(
        super::super::bindings::baseline(&cold).unwrap().unwrap().0,
        id
    );
    let report = super::super::history::run(super::super::history::Command::Show {
        root: copy.path().to_owned(),
        id: id.clone(),
        kind: "bindings-closure".into(),
        cursor: None,
        limit: 100,
    })
    .unwrap();
    assert_eq!(report["status"], "FROZEN_SNAPSHOT");
    assert!(!report["items"].as_array().unwrap().is_empty());
    let files = export_files(&cold, &raw, None).unwrap();
    assert_eq!(
        resolve_export(&files, "").unwrap(),
        resolve(&cold, &raw).unwrap().0
    );
    assert!(cold.lock().is_err());
    assert!(cold.atomic("docs/forbidden", b"no").is_err());
    assert!(!copy.path().join(".codeclew").exists());
    let reference = closure(&raw).unwrap().remove(0);
    fs::remove_file(cold.path(&object_path(&reference).unwrap()).unwrap()).unwrap();
    assert!(super::super::bindings::baseline(&cold).is_err());
    let report = super::super::history::run(super::super::history::Command::Show {
        root: copy.path().to_owned(),
        id,
        kind: "summary".into(),
        cursor: None,
        limit: 100,
    })
    .unwrap();
    assert_eq!(report["status"], "DAMAGED_OR_EXPIRED");
    assert!(!copy.path().join(".codeclew").exists());
}

#[test]
fn remote_repacking_omits_unused_text_from_a_reused_physical_chunk() {
    let (_temp, repo) = repository();
    let sentinel = "UNUSED_SOURCE_TEXT_MUST_NEVER_REACH_REMOTE_DOWNLOADS";
    let mut original = logical("SOURCE_SYNTAX");
    let mut unused = original["retainedSources"]["source"].clone();
    unused["id"] = json!("unused");
    unused["text"] = json!(sentinel);
    unused["textDigest"] = json!(canonical::hash_bytes(sentinel.as_bytes()));
    original["retainedSources"]["unused"] = unused;
    let first = encode(&repo, original.clone(), None, false).unwrap();
    first.persist(&repo).unwrap();
    original["retainedSources"]
        .as_object_mut()
        .unwrap()
        .remove("unused");
    let selected = encode(&repo, original.clone(), Some(&first.manifest), false).unwrap();
    selected.persist(&repo).unwrap();
    let embedded = export_files(&repo, &selected.manifest, None).unwrap();
    assert!(
        embedded
            .values()
            .any(|raw| String::from_utf8_lossy(raw).contains(sentinel))
    );
    assert!(
        !bytes(&resolve_export(&embedded, "").unwrap())
            .unwrap()
            .windows(sentinel.len())
            .any(|w| w == sentinel.as_bytes())
    );
    original["retainedSources"]["source"]["text"] = json!("");
    original["fragments"]["service:orders/original"]["evidence"]["sources"]["source"]["text"] =
        json!("");
    let mut remote = export_files(&repo, &selected.manifest, Some(original.clone())).unwrap();
    assert!(
        remote
            .values()
            .all(|raw| !String::from_utf8_lossy(raw).contains(sentinel))
    );
    assert_eq!(resolve_export(&remote, "").unwrap(), original);
    assert!(resolve_with(&remote["frozen-bindings.json"], false, |_| unreachable!()).is_err());
    let object = remote
        .keys()
        .find(|name| name.starts_with(OBJECT_ROOT))
        .unwrap()
        .clone();
    remote.insert(object, b"tampered".to_vec());
    assert!(resolve_export(&remote, "").is_err());
}

#[test]
fn malformed_closures_are_rejected_before_hydration_and_symlinks_are_unsafe() {
    let (_temp, repo) = repository();
    let encoded = encode(&repo, logical("SOURCE_SYNTAX"), None, false).unwrap();
    let original: Value = serde_json::from_slice(&encoded.manifest).unwrap();
    for case in [
        "path",
        "duplicate",
        "size",
        "logical",
        "schema",
        "unlisted-index",
    ] {
        let mut m = original.clone();
        match case {
            "path" => m["objectRoot"] = json!("../private"),
            "duplicate" => {
                let r = m["objects"][0].clone();
                m["objects"].as_array_mut().unwrap().push(r);
            }
            "size" => m["objects"][0]["size"] = json!(MAX + 1),
            "logical" => m["logicalBytes"] = json!(MAX + 1),
            "schema" => m["objects"][0]["schema"] = json!("private-cache/1"),
            "unlisted-index" => {
                m["partitions"][0]["index"]["digest"] = json!(format!("sha256:{}", "f".repeat(64)))
            }
            _ => unreachable!(),
        }
        assert!(
            resolve_with(&bytes(&m).unwrap(), false, |_| panic!("hydrated {case}")).is_err(),
            "{case}"
        );
    }
    encoded.persist(&repo).unwrap();
    let r = &encoded.references[0];
    let object = repo.path(&object_path(r).unwrap()).unwrap();
    let saved = fs::read(&object).unwrap();
    fs::remove_file(&object).unwrap();
    let external = repo.root.join("outside.json");
    fs::write(&external, saved).unwrap();
    std::os::unix::fs::symlink(&external, &object).unwrap();
    assert!(resolve(&repo, &encoded.manifest).is_err());
}

#[test]
fn duplicate_or_wrong_historical_memberships_fail_even_with_rehashed_objects() {
    let (_temp, repo) = repository();
    let encoded = encode(&repo, logical("SOURCE_SYNTAX"), None, false).unwrap();
    for case in ["duplicate", "wrong-owner", "cycle", "wrong-occurrence"] {
        let mut m: Manifest = parse(&encoded.manifest).unwrap();
        let partition = m
            .partitions
            .iter_mut()
            .find(|p| p.owner.kind == "subject")
            .unwrap();
        let old = partition.index.clone();
        let mut index: Index = parse(&encoded.new_objects[&old.digest]).unwrap();
        match case {
            "duplicate" => index.members.push(index.members[0].clone()),
            "wrong-owner" => index.owner.id = "service:someone-else".into(),
            "cycle" => index.members[0].payload.chunk = old.clone(),
            "wrong-occurrence" => {
                let member = index
                    .members
                    .iter_mut()
                    .find(|m| m.table == "fragmentSources")
                    .unwrap();
                // Cross-wire metadata to a current occurrence while keeping the old text.
                let current: Index = encoded
                    .references
                    .iter()
                    .filter(|r| r.schema == PARTITION)
                    .map(|r| parse::<Index>(&encoded.new_objects[&r.digest]).unwrap())
                    .find(|i| i.members.iter().any(|m| m.table == "retainedSources"))
                    .unwrap();
                member.payload = current
                    .members
                    .iter()
                    .find(|m| m.table == "retainedSources")
                    .unwrap()
                    .payload
                    .clone();
            }
            _ => unreachable!(),
        }
        let raw = bytes(&index).unwrap();
        partition.index = ObjectRef {
            schema: PARTITION.into(),
            digest: canonical::hash_bytes(&raw),
            size: raw.len() as u64,
        };
        let new_reference = partition.index.clone();
        *m.objects
            .iter_mut()
            .find(|r| r.digest == old.digest)
            .unwrap() = new_reference.clone();
        let result = resolve_with(&bytes(&m).unwrap(), false, |r| {
            Ok(if r.digest == new_reference.digest {
                raw.clone()
            } else {
                encoded.new_objects[&r.digest].clone()
            })
        });
        assert!(result.is_err(), "{case}");
    }
}

#[test]
fn real_structured_overbudget_publication_preserves_last_good_pointer() {
    let (_temp, repo) = repository();
    let id = publish(&repo, logical("SOURCE_SYNTAX"), 1);
    let pointer = fs::read(repo.path("docs/index.html").unwrap()).unwrap();
    let mut value = logical("SOURCE_SYNTAX");
    let spec: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/bindings-storage-scale/spec.json"
    ))
    .unwrap();
    for i in 0..spec["overBudget"]["structuredSourceCount"]
        .as_u64()
        .unwrap()
    {
        let text = format!("// structured source {i}\n")
            .chars()
            .cycle()
            .take(spec["overBudget"]["sourceTextBytes"].as_u64().unwrap() as usize)
            .collect::<String>();
        let mut source = value["retainedSources"]["source"].clone();
        source["id"] = json!(format!("large-{i}"));
        source["textDigest"] = json!(canonical::hash_bytes(text.as_bytes()));
        source["text"] = json!(text);
        value["retainedSources"][format!("large-{i}")] = source;
    }
    let err = match encode(&repo, value, None, false) {
        Ok(_) => panic!("structured overbudget input accepted"),
        Err(e) => e,
    };
    assert_eq!(err.code, ErrorCode::SliceBudgetExceeded);
    assert_eq!(
        fs::read(repo.path("docs/index.html").unwrap()).unwrap(),
        pointer
    );
    assert_eq!(
        super::super::bindings::baseline(&repo).unwrap().unwrap().0,
        id
    );
}
