//! Original compilation-unit text, independently of declaration/entrypoint semantics.
use super::*;
use crate::{csharp_adapter_v2::CSharpCompilerFact, repository_snapshot::RepositoryInputSnapshot};

fn membership(fact: &Value, binding: &str) -> Result<(), ClewError> {
    let raw = json!({"schema":fact["schema"],"kind":fact["kind"],"file":fact["file"],
        "sourceContentDigest":fact["sourceContentDigest"],"resolution":fact["resolution"]});
    if fact["schema"] != super::super::csharp::CSHARP_FACT_SCHEMA
        || fact["resolution"] != "SOURCE_MEMBERSHIP_EXACT"
        || !matches!(
            serde_json::from_value::<CSharpCompilerFact>(raw.clone()),
            Ok(CSharpCompilerFact::SourceFile { .. })
        )
        || crate::cas::CasObject::for_bytes(
            super::super::csharp::CSHARP_FACT_SCHEMA,
            &bytes(&raw)?,
        )?
        .digest
            != binding
    {
        return Err(invalid("C# compilation-unit membership binding is invalid"));
    }
    Ok(())
}

pub(super) fn project_file(
    evidence: &mut ServiceEvidence,
    service: &Service,
    sources: &CompilationSource,
    known: &BTreeSet<String>,
    fact: &Value,
    binding: &str,
) -> Result<bool, ClewError> {
    if service.language != "csharp" || fact["kind"] != "SOURCE_FILE" {
        return Ok(false);
    }
    membership(fact, binding)?;
    let scope = resolve_scope_key(&fact["scope"], known)?;
    if sources.transformed.get(&scope) != Some(&false) {
        return Err(invalid(
            "C# original compilation unit requires untransformed snapshot text",
        ));
    }
    let file = fact["file"]
        .as_str()
        .ok_or_else(|| invalid("C# compilation-unit file is missing"))?;
    store::relative(file)?;
    let blob = sources
        .blob(&scope, file)
        .ok_or_else(|| invalid("C# admitted compilation-unit text is unavailable or not UTF-8"))?;
    if fact["sourceContentDigest"] != blob.content_digest {
        return Err(invalid(
            "C# compilation-unit content differs from sealed membership",
        ));
    }
    let identity = scoped_identity(&scope, &format!("compilation-unit:{file}"));
    let mut source_ids = Vec::new();
    if !blob.text.is_empty() {
        let id = source_id(&service.id, &identity)?;
        evidence.sources.insert(
            id.clone(),
            Source {
                id: id.clone(),
                service: service.id.clone(),
                revision: evidence.revision.clone(),
                file: file.into(),
                start_line: 1,
                end_line: blob.text.lines().count() as u64,
                text: blob.text.to_string(),
                text_digest: blob.content_digest.clone(),
                evidence_digest: binding.into(),
                authority: "EXACT_SNAPSHOT_TEXT".into(),
                text_origin: None,
                occurrence: None,
                url: None,
            },
        );
        source_ids.push(id);
    }
    let mut normalized = fact.clone();
    normalized["scope"] = json!(scope);
    normalized["compilerFactBinding"] = json!(binding);
    normalized["sourceStatus"] = json!(if source_ids.is_empty() {
        "EMPTY_SOURCE_FILE"
    } else {
        "SOURCE_RETAINED"
    });
    normalized["sourceRange"] = json!({"byteStart":0,"byteEnd":blob.text.len()});
    normalized["meaning"] = json!("ORIGINAL_FILE_TEXT_NOT_ENTRYPOINT_OR_RUNTIME_PROOF");
    let id = dependency_id(&service.id, "compilation-source", &identity)?;
    if evidence.observations.contains_key(&id) {
        return Err(invalid("duplicate C# compilation-unit membership"));
    }
    evidence.observations.insert(
        id.clone(),
        Observation {
            id,
            kind: "SOURCE_FILE".into(),
            service: service.id.clone(),
            symbol: file.into(),
            digest: digest(&normalized)?,
            normalized,
            source_ids,
        },
    );
    Ok(true)
}

/// Bind file evidence to the real sealed worktree blob, not an inferred Git origin.
/// No URL or committed-text claim is added for generated/untracked/dirty inputs.
pub(super) fn bind_snapshot(
    evidence: &mut ServiceEvidence,
    snapshot: &RepositoryInputSnapshot,
) -> Result<(), ClewError> {
    let files: BTreeMap<_, _> = snapshot
        .worktree
        .iter()
        .map(|entry| (entry.path.as_str(), entry))
        .collect();
    for observation in evidence.observations.values_mut().filter(|o| {
        o.kind == "SOURCE_FILE"
            && o.normalized["schema"] == super::super::csharp::CSHARP_FACT_SCHEMA
    }) {
        let file = observation.normalized["file"]
            .as_str()
            .ok_or_else(|| invalid("C# original file path is missing"))?;
        let entry = files
            .get(file)
            .filter(|entry| entry.kind == crate::repository_snapshot::WorktreeKind::Regular)
            .ok_or_else(|| {
                invalid("C# original compilation unit is outside regular sealed snapshot inputs")
            })?;
        let reference = entry
            .content
            .as_ref()
            .ok_or_else(|| invalid("C# original snapshot blob is unavailable"))?;
        let text = match observation.source_ids.as_slice() {
            [] if observation.normalized["sourceStatus"] == "EMPTY_SOURCE_FILE" => "",
            [id] => {
                let source = evidence
                    .sources
                    .get(id)
                    .ok_or_else(|| invalid("C# original file SOURCE is unavailable"))?;
                if source.service != evidence.service
                    || source.revision != evidence.revision
                    || source.file != file
                {
                    return Err(invalid("C# original file SOURCE scope changed"));
                }
                source.text.as_str()
            }
            _ => {
                return Err(invalid(
                    "C# original compilation-unit SOURCE binding is ambiguous",
                ));
            }
        };
        if crate::cas::CasObject::for_bytes(&reference.object_schema, text.as_bytes())?
            != *reference
        {
            return Err(invalid(
                "C# original file differs from its sealed snapshot blob",
            ));
        }
        let occurrence = SourceOccurrence {
            snapshot: snapshot.snapshot_id.clone(),
            blob: reference.digest.clone(),
            start_byte: 0,
            end_byte: text.len(),
            generated_origin: None,
        };
        if let [id] = observation.source_ids.as_slice() {
            let source = evidence.sources.get_mut(id).unwrap();
            source.evidence_digest = digest(&(
                SOURCE_EXTRACTOR,
                &occurrence.snapshot,
                &occurrence.blob,
                0usize,
                occurrence.end_byte,
            ))?;
            source.occurrence = Some(occurrence);
        }
        observation.normalized["snapshotSource"] =
            json!({"snapshot":snapshot.snapshot_id,"blob":reference});
        observation.digest = digest(&observation.normalized)?;
    }
    Ok(())
}

pub(super) fn verify_file(
    evidence: &ServiceEvidence,
    observation: &Observation,
) -> Result<(), ClewError> {
    if observation.kind != "SOURCE_FILE"
        || observation.normalized["schema"] != super::super::csharp::CSHARP_FACT_SCHEMA
    {
        return Ok(());
    }
    let n = &observation.normalized;
    membership(n, n["compilerFactBinding"].as_str().unwrap_or_default())?;
    if observation.service != evidence.service || n["scope"].as_str().is_none_or(str::is_empty) {
        return Err(invalid("C# compilation-unit scope is invalid"));
    }
    let identity = scoped_identity(
        n["scope"].as_str().unwrap(),
        &format!(
            "compilation-unit:{}",
            n["file"].as_str().unwrap_or_default()
        ),
    );
    if observation.id != dependency_id(&evidence.service, "compilation-source", &identity)? {
        return Err(invalid(
            "C# compilation-unit identity differs from its scope",
        ));
    }
    let text = match observation.source_ids.as_slice() {
        [] if n["sourceStatus"] == "EMPTY_SOURCE_FILE" => "",
        [id] if n["sourceStatus"] == "SOURCE_RETAINED" => {
            let source = evidence
                .sources
                .get(id)
                .ok_or_else(|| invalid("C# compilation-unit source is missing"))?;
            if source.id != source_id(&evidence.service, &identity)?
                || source.service != evidence.service
                || source.revision != evidence.revision
                || n["file"] != source.file
                || source.authority != "EXACT_SNAPSHOT_TEXT"
                || source.start_line != 1
                || source.end_line != source.text.lines().count() as u64
            {
                return Err(invalid("C# compilation-unit source metadata changed"));
            }
            source.text.as_str()
        }
        _ => return Err(invalid("C# compilation-unit source choice changed")),
    };
    if n["sourceContentDigest"] != canonical::hash_bytes(text.as_bytes())
        || n["sourceRange"] != json!({"byteStart":0,"byteEnd":text.len()})
    {
        return Err(invalid(
            "C# compilation-unit full-file range or digest changed",
        ));
    }
    if let Some(pin) = n.get("snapshotSource") {
        let reference: crate::cas::CasObject =
            serde_json::from_value(pin["blob"].clone()).map_err(io_error)?;
        if crate::cas::CasObject::for_bytes(&reference.object_schema, text.as_bytes())? != reference
        {
            return Err(invalid("C# compilation-unit snapshot blob changed"));
        }
        if let [id] = observation.source_ids.as_slice() {
            let expected = SourceOccurrence {
                snapshot: pin["snapshot"].as_str().unwrap_or_default().into(),
                blob: reference.digest,
                start_byte: 0,
                end_byte: text.len(),
                generated_origin: None,
            };
            if evidence.sources[id].occurrence.as_ref() != Some(&expected) {
                return Err(invalid("C# compilation-unit original occurrence changed"));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repository_snapshot::{SNAPSHOT_SCHEMA, WorktreeEntry, WorktreeKind};

    fn service() -> Service {
        serde_json::from_value(json!({"schema":"codeclew-documentation-service/1.0",
            "id":"owned","title":"Owned","repositoryId":"owned","repository":"https://example.invalid/owned",
            "language":"csharp","profile":"csharp-dotnet-msbuild-read-only","targetRef":"main",
            "compilations":["csproj:Owned.csproj@net10.0"]})).unwrap()
    }

    fn fact(scope: &str, file: &str, text: &str) -> (Value, String) {
        let mut raw = json!({"schema":super::super::super::csharp::CSHARP_FACT_SCHEMA,"kind":"SOURCE_FILE",
            "file":file,"sourceContentDigest":canonical::hash_bytes(text.as_bytes()),"resolution":"SOURCE_MEMBERSHIP_EXACT"});
        let binding = crate::cas::CasObject::for_bytes(
            super::super::super::csharp::CSHARP_FACT_SCHEMA,
            &bytes(&raw).unwrap(),
        )
        .unwrap()
        .digest;
        raw["scope"] = json!({"compilation":scope});
        (raw, binding)
    }

    fn sources(
        scope: &str,
        files: &BTreeMap<String, String>,
        transformed: bool,
    ) -> CompilationSource {
        CompilationSource {
            contents: BTreeMap::from([(
                scope.into(),
                source_table_from_text(
                    &files
                        .iter()
                        .map(|(p, t)| (p.clone(), Arc::<str>::from(t.as_str())))
                        .collect(),
                ),
            )]),
            transformed: BTreeMap::from([(scope.into(), transformed)]),
            contracts: BTreeMap::new(),
        }
    }

    fn evidence(files: &BTreeMap<String, String>) -> ServiceEvidence {
        let scope = "csproj:Owned.csproj@net10.0";
        project_scoped(
            &service(),
            &"a".repeat(40),
            "service",
            "DEVELOPMENT",
            "PARTIAL",
            files.iter().map(|(p, t)| fact(scope, p, t)).collect(),
            &sources(scope, files, false),
            &BTreeSet::from([scope.into()]),
        )
        .unwrap()
    }

    fn snapshot(files: &BTreeMap<String, String>) -> RepositoryInputSnapshot {
        RepositoryInputSnapshot {
            schema: SNAPSHOT_SCHEMA.into(),
            snapshot_id: format!("sha256:{}", "b".repeat(64)),
            staged_view_digest: String::new(),
            cached_view_digest: String::new(),
            untracked_view_digest: String::new(),
            index: vec![],
            worktree: files
                .iter()
                .map(|(file, text)| WorktreeEntry {
                    path: file.clone(),
                    kind: WorktreeKind::Regular,
                    mode: 0o100644,
                    content: Some(
                        crate::cas::CasObject::for_bytes(
                            "owned-snapshot-blob/1.0",
                            text.as_bytes(),
                        )
                        .unwrap(),
                    ),
                })
                .collect(),
        }
    }

    #[test]
    fn full_units_preserve_bom_crlf_unicode_and_top_level_without_synthetic_symbols() {
        let files = BTreeMap::from([("Bootstrap.cs".into(), "\u{feff}using System;\r\n// π🙂\r\nConsole.WriteLine(new Clock().Read());\r\n// tail\r\n".into()),
            ("Clock.cs".into(), "// before\r\nclass Clock { public string Read() => \"ready\"; }\r\n// after\r\n".into())]);
        let mut e = evidence(&files);
        bind_snapshot(&mut e, &snapshot(&files)).unwrap();
        verify_evidence(&e).unwrap();
        assert_eq!(e.sources.len(), 2);
        assert_eq!(
            e.observations
                .values()
                .filter(|o| o.kind == "SOURCE_FILE")
                .count(),
            2
        );
        assert!(e.entrypoints.is_empty());
        for source in e.sources.values() {
            assert_eq!(source.text, files[&source.file]);
            assert_eq!(source.start_line, 1);
            let range = source.occurrence.as_ref().unwrap();
            assert_eq!((range.start_byte, range.end_byte), (0, source.text.len()));
            assert!(source.url.is_none());
            assert!(source.text_origin.is_none());
        }
        assert!(
            e.observations
                .values()
                .all(|o| matches!(o.kind.as_str(), "SOURCE_FILE" | "CONTRACT_SCOPE"))
        );
        let temp = tempfile::tempdir().unwrap();
        Repository::init(temp.path(), "Owned").unwrap();
        let repo = Repository::open(temp.path()).unwrap();
        let stored = super::super::super::cache::store_capture(&repo, &e).unwrap();
        let loaded = super::super::super::cache::load_capture(&repo, &stored).unwrap();
        assert_eq!(loaded.sources, e.sources);
    }

    #[test]
    fn empty_units_are_explicit_metadata_without_a_fake_nonempty_source() {
        let mut e = evidence(&BTreeMap::from([("Empty.cs".into(), String::new())]));
        bind_snapshot(
            &mut e,
            &snapshot(&BTreeMap::from([("Empty.cs".into(), String::new())])),
        )
        .unwrap();
        verify_evidence(&e).unwrap();
        assert!(e.sources.is_empty());
        let o = e.observations.values().next().unwrap();
        assert_eq!(o.normalized["sourceStatus"], "EMPTY_SOURCE_FILE");
        assert!(o.source_ids.is_empty());
    }

    #[test]
    fn ordinary_declaration_source_stays_intact_beside_the_complete_unit() {
        let scope = "csproj:Owned.csproj@net10.0";
        let text = "// before\nclass Clock {}\n// after\n";
        let files = BTreeMap::from([("Clock.cs".into(), text.into())]);
        let raw = json!({"schema":super::super::super::csharp::CSHARP_FACT_SCHEMA,
            "kind":"DECLARATION","declarationKind":"CLASS","symbolIdentity":"class:Clock",
            "name":"Clock","ownerIdentity":"module:unnamed","file":"Clock.cs",
            "startLine":2,"endLine":2,"resolution":"COMPILER_EXACT"});
        let binding = crate::cas::CasObject::for_bytes(
            super::super::super::csharp::CSHARP_FACT_SCHEMA,
            &bytes(&raw).unwrap(),
        )
        .unwrap()
        .digest;
        let mut declaration = raw;
        declaration["scope"] = json!({"compilation":scope});
        let facts = super::super::super::csharp::project_facts(vec![
            fact(scope, "Clock.cs", text),
            (declaration, binding),
        ])
        .unwrap();
        let e = project_scoped(
            &service(),
            &"a".repeat(40),
            "service",
            "DEVELOPMENT",
            "PARTIAL",
            facts,
            &sources(scope, &files, false),
            &BTreeSet::from([scope.into()]),
        )
        .unwrap();
        let declaration = e
            .observations
            .values()
            .find(|o| o.kind == "SYMBOL")
            .unwrap();
        assert_eq!(e.sources[&declaration.source_ids[0]].text, "class Clock {}");
        let unit = e
            .observations
            .values()
            .find(|o| o.kind == "SOURCE_FILE")
            .unwrap();
        assert_eq!(e.sources[&unit.source_ids[0]].text, text);
        assert_eq!(
            e.observations
                .values()
                .filter(|o| o.kind == "SYMBOL")
                .count(),
            1
        );
    }

    #[test]
    fn same_path_under_distinct_compilations_keeps_distinct_source_provenance() {
        let files = BTreeMap::from([("Shared.cs".into(), "// original\n".into())]);
        let mut table = sources("first", &files, false);
        let second = sources("second", &files, false);
        table.contents.extend(second.contents);
        table.transformed.extend(second.transformed);
        let mut e = project_scoped(
            &service(),
            &"a".repeat(40),
            "s",
            "DEVELOPMENT",
            "PARTIAL",
            vec![
                fact("first", "Shared.cs", &files["Shared.cs"]),
                fact("second", "Shared.cs", &files["Shared.cs"]),
            ],
            &table,
            &BTreeSet::from(["first".into(), "second".into()]),
        )
        .unwrap();
        bind_snapshot(&mut e, &snapshot(&files)).unwrap();
        verify_evidence(&e).unwrap();
        assert_eq!(e.sources.len(), 2);
        assert_eq!(
            e.observations
                .values()
                .filter(|o| o.kind == "SOURCE_FILE")
                .count(),
            2
        );
        assert!(
            e.observations
                .values()
                .all(|o| matches!(o.kind.as_str(), "SOURCE_FILE" | "CONTRACT_SCOPE"))
        );
        let ids = e
            .observations
            .values()
            .flat_map(|o| o.source_ids.iter().cloned())
            .collect::<BTreeSet<_>>();
        assert_eq!(ids.len(), 2);
        assert_eq!(
            e.observations
                .values()
                .filter(|o| o.kind == "SOURCE_FILE")
                .map(|o| o.normalized["scope"].as_str().unwrap())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["first", "second"])
        );
        for source in e.sources.values() {
            assert_eq!(source.text, files["Shared.cs"]);
            assert_eq!(
                source.occurrence.as_ref().unwrap().snapshot,
                snapshot(&files).snapshot_id
            );
        }
        let temp = tempfile::tempdir().unwrap();
        Repository::init(temp.path(), "Owned").unwrap();
        let repo = Repository::open(temp.path()).unwrap();
        let stored = super::super::super::cache::store_capture(&repo, &e).unwrap();
        let loaded = super::super::super::cache::load_capture(&repo, &stored).unwrap();
        verify_evidence(&loaded).unwrap();
        assert_eq!(loaded.sources, e.sources);
        assert_eq!(loaded.observations, e.observations);
    }

    #[test]
    fn different_bytes_for_one_path_cannot_bind_to_one_original_snapshot() {
        let files = BTreeMap::from([("Shared.cs".into(), "// original\n".into())]);
        let other = BTreeMap::from([("Shared.cs".into(), "// different\n".into())]);
        let mut table = sources("first", &files, false);
        let second = sources("second", &other, false);
        table.contents.extend(second.contents);
        table.transformed.extend(second.transformed);
        let mut e = project_scoped(
            &service(),
            &"a".repeat(40),
            "s",
            "DEVELOPMENT",
            "PARTIAL",
            vec![
                fact("first", "Shared.cs", &files["Shared.cs"]),
                fact("second", "Shared.cs", &other["Shared.cs"]),
            ],
            &table,
            &BTreeSet::from(["first".into(), "second".into()]),
        )
        .unwrap();
        assert!(bind_snapshot(&mut e, &snapshot(&files)).is_err());
    }

    #[test]
    fn membership_digest_scope_missing_text_and_transformed_inputs_fail_closed() {
        let files: BTreeMap<String, String> =
            BTreeMap::from([("Unit.cs".into(), "// π🙂\n".into())]);
        for bad in ["digest", "binding", "scope", "missing", "transformed"] {
            let scope = "csproj:Owned.csproj@net10.0";
            let (mut f, mut binding) = fact(scope, "Unit.cs", &files["Unit.cs"]);
            if bad == "digest" {
                f["sourceContentDigest"] = json!(canonical::hash_bytes(b"other"));
                binding=crate::cas::CasObject::for_bytes(super::super::super::csharp::CSHARP_FACT_SCHEMA,
                &bytes(&json!({"schema":f["schema"],"kind":f["kind"],"file":f["file"],"sourceContentDigest":f["sourceContentDigest"],"resolution":f["resolution"]})).unwrap()).unwrap().digest;
            }
            if bad == "binding" {
                binding = "wrong".into();
            }
            if bad == "scope" {
                f["scope"] = json!({"compilation":"foreign"});
            }
            let empty_files = BTreeMap::new();
            let table = sources(
                scope,
                if bad == "missing" {
                    &empty_files
                } else {
                    &files
                },
                bad == "transformed",
            );
            assert!(
                project_scoped(
                    &service(),
                    &"a".repeat(40),
                    "s",
                    "DEVELOPMENT",
                    "PARTIAL",
                    vec![(f, binding)],
                    &table,
                    &BTreeSet::from([scope.into()])
                )
                .is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn original_snapshot_requires_exact_regular_blob_and_matching_service_revision() {
        let files = BTreeMap::from([("Unit.cs".into(), "// original\n".into())]);
        for bad in ["missing", "symlink", "bytes", "service", "revision"] {
            let mut e = evidence(&files);
            let mut pin = snapshot(&files);
            match bad {
                "missing" => pin.worktree.clear(),
                "symlink" => pin.worktree[0].kind = WorktreeKind::Symlink,
                "bytes" => {
                    pin.worktree[0].content = Some(
                        crate::cas::CasObject::for_bytes("owned-snapshot-blob/1.0", b"other")
                            .unwrap(),
                    )
                }
                "service" => e.sources.values_mut().next().unwrap().service = "foreign".into(),
                _ => e.sources.values_mut().next().unwrap().revision = "c".repeat(40),
            }
            assert!(bind_snapshot(&mut e, &pin).is_err(), "{bad}");
        }
    }

    #[test]
    fn rehashed_file_observation_cannot_change_original_bytes_ranges_or_compilation() {
        let files = BTreeMap::from([("Unit.cs".into(), "// original\n".into())]);
        let mut original = evidence(&files);
        bind_snapshot(&mut original, &snapshot(&files)).unwrap();
        for field in [
            "scope",
            "sourceRange",
            "sourceContentDigest",
            "snapshotSource",
        ] {
            let mut e = original.clone();
            let o = e.observations.values_mut().next().unwrap();
            o.normalized[field] = json!("changed");
            o.digest = digest(&o.normalized).unwrap();
            assert!(verify_evidence(&e).is_err(), "{field}");
        }
    }
}
