//! Content-addressed persistence for the immutable inputs captured by a docs check.
use super::{
    cache, check, digest, invalid,
    store::{self, Repository, RepositoryInputs},
};
use crate::error::{ClewError, ErrorCode};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub(super) const MANIFEST_SCHEMA: &str = "codeclew-documentation-source-inputs-manifest/1.0";
pub(super) const DECLARATION_MANIFEST_SCHEMA: &str =
    "codeclew-documentation-declaration-inputs-manifest/1.0";

const SERVICE_SCHEMA: &str = "codeclew-documentation-source-input-service/1.0";
const INTERACTION_SCHEMA: &str = "codeclew-documentation-source-input-interaction/1.0";
const SCENARIO_SCHEMA: &str = "codeclew-documentation-source-input-scenario/1.0";
const ENTITY_SCHEMA: &str = "codeclew-documentation-source-input-entity/1.0";
const NOTE_METADATA_SCHEMA: &str = "codeclew-documentation-source-input-note-metadata/1.0";
const NOTE_ORIGINAL_SCHEMA: &str = "codeclew-documentation-source-input-note-original/1.0";
const EXPECTATION_SCHEMA: &str = "codeclew-documentation-source-input-expectation/1.0";
const UPDATE_POLICY_SCHEMA: &str = "codeclew-documentation-source-input-update-policy/1.0";
const UPDATE_EVENT_SCHEMA: &str = "codeclew-documentation-source-input-update-event/1.0";
const UPDATE_STATE_SCHEMA: &str = "codeclew-documentation-update-state/1.0";
const MAX_ITEMS: usize = store::MAX_RECORDS;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NoteReferences {
    metadata: cache::ObjectRef,
    original: cache::ObjectRef,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateStateReferences {
    schema: String,
    targets: BTreeMap<String, cache::ObjectRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Manifest {
    schema: String,
    input_digest: String,
    manifest: super::model::Manifest,
    selected_services: BTreeSet<String>,
    services: BTreeMap<String, cache::ObjectRef>,
    interactions: BTreeMap<String, cache::ObjectRef>,
    scenarios: BTreeMap<String, cache::ObjectRef>,
    entities: BTreeMap<String, cache::ObjectRef>,
    notes: BTreeMap<String, NoteReferences>,
    evidence_expectations: BTreeMap<String, cache::ObjectRef>,
    update_policies: BTreeMap<String, cache::ObjectRef>,
    update_state: UpdateStateReferences,
}

fn state_corrupt(message: &'static str) -> ClewError {
    ClewError::new(ErrorCode::StateCorrupt, message)
}

fn check_reference(reference: &cache::ObjectRef, schema: &str) -> Result<(), ClewError> {
    if reference.schema != schema {
        return Err(invalid(
            "source input object schema does not match its field",
        ));
    }
    Ok(())
}

fn store_map<T: Serialize>(
    repo: &Repository,
    schema: &str,
    values: &BTreeMap<String, T>,
) -> Result<BTreeMap<String, cache::ObjectRef>, ClewError> {
    if values.len() > MAX_ITEMS {
        return Err(invalid("source input map exceeds its record bound"));
    }
    values
        .iter()
        .map(|(id, value)| {
            if !store::valid_id(id) {
                return Err(invalid("source input map contains an invalid record ID"));
            }
            Ok((id.clone(), cache::put_json(repo, schema, value)?))
        })
        .collect()
}

fn load_map<T: DeserializeOwned>(
    repo: &Repository,
    schema: &str,
    references: &BTreeMap<String, cache::ObjectRef>,
) -> Result<BTreeMap<String, T>, ClewError> {
    if references.len() > MAX_ITEMS {
        return Err(invalid("source input map exceeds its record bound"));
    }
    references
        .iter()
        .map(|(id, reference)| {
            if !store::valid_id(id) {
                return Err(invalid("source input map contains an invalid record ID"));
            }
            check_reference(reference, schema)?;
            let value = cache::get_json(repo, reference, check::PORTABLE_CACHE_MAX_BYTES)?
                .ok_or_else(|| state_corrupt("source input object is missing"))?;
            Ok((id.clone(), value))
        })
        .collect()
}

fn store_notes(
    repo: &Repository,
    notes: &BTreeMap<String, Value>,
) -> Result<BTreeMap<String, NoteReferences>, ClewError> {
    if notes.len() > MAX_ITEMS {
        return Err(invalid("source input map exceeds its record bound"));
    }
    notes
        .iter()
        .map(|(id, note)| {
            if !store::valid_id(id) {
                return Err(invalid("source input map contains an invalid record ID"));
            }
            let object = note
                .as_object()
                .ok_or_else(|| invalid("note snapshot metadata must be an object"))?;
            let original = object
                .get("original")
                .ok_or_else(|| invalid("note snapshot has no original content"))?;
            let mut metadata = object.clone();
            if metadata.remove("original").is_none() {
                return Err(invalid("note snapshot metadata has no original split"));
            }
            Ok((
                id.clone(),
                NoteReferences {
                    metadata: cache::put_json(
                        repo,
                        NOTE_METADATA_SCHEMA,
                        &Value::Object(metadata),
                    )?,
                    original: cache::put_json(repo, NOTE_ORIGINAL_SCHEMA, original)?,
                },
            ))
        })
        .collect()
}

fn load_notes(
    repo: &Repository,
    references: &BTreeMap<String, NoteReferences>,
) -> Result<BTreeMap<String, Value>, ClewError> {
    if references.len() > MAX_ITEMS {
        return Err(invalid("source input map exceeds its record bound"));
    }
    references
        .iter()
        .map(|(id, references)| {
            if !store::valid_id(id) {
                return Err(invalid("source input map contains an invalid record ID"));
            }
            check_reference(&references.metadata, NOTE_METADATA_SCHEMA)?;
            check_reference(&references.original, NOTE_ORIGINAL_SCHEMA)?;
            let metadata = cache::get_json::<Value>(
                repo,
                &references.metadata,
                check::PORTABLE_CACHE_MAX_BYTES,
            )?
            .ok_or_else(|| state_corrupt("note metadata object is missing"))?;
            let original = cache::get_json::<Value>(
                repo,
                &references.original,
                check::PORTABLE_CACHE_MAX_BYTES,
            )?
            .ok_or_else(|| state_corrupt("note original object is missing"))?;
            let mut metadata = metadata
                .as_object()
                .cloned()
                .ok_or_else(|| invalid("note snapshot metadata must be an object"))?;
            if metadata.contains_key("original") {
                return Err(invalid("note snapshot metadata already contains original"));
            }
            metadata.insert("original".into(), original);
            Ok((id.clone(), Value::Object(metadata)))
        })
        .collect()
}

fn validate_keyed_inputs(inputs: &RepositoryInputs) -> Result<(), ClewError> {
    for (id, service) in &inputs.services {
        if id != &service.id {
            return Err(invalid("source input service key does not match its ID"));
        }
    }
    for (id, interaction) in &inputs.interactions {
        if id != &interaction.id {
            return Err(invalid(
                "source input interaction key does not match its ID",
            ));
        }
    }
    for (id, scenario) in &inputs.scenarios {
        if id != &scenario.id {
            return Err(invalid("source input scenario key does not match its ID"));
        }
    }
    for (id, entity) in &inputs.entities {
        if id != &entity.id {
            return Err(invalid("source input entity key does not match its ID"));
        }
    }
    for (id, expectation) in &inputs.evidence_expectations {
        if id != &expectation.service {
            return Err(invalid(
                "source input expectation key does not match its service",
            ));
        }
    }
    for (id, policy) in &inputs.update_policies {
        if id != &policy.service {
            return Err(invalid(
                "source input update policy key does not match its service",
            ));
        }
    }
    for (id, event) in &inputs.update_state.targets {
        if id != &event.service {
            return Err(invalid(
                "source input update target key does not match its service",
            ));
        }
    }
    Ok(())
}

pub(super) fn validate(inputs: &check::SourceInputs) -> Result<(), ClewError> {
    if inputs.schema != check::SOURCE_INPUTS_SCHEMA
        || inputs.inputs.update_state.schema != UPDATE_STATE_SCHEMA
        || digest(&inputs.inputs)? != inputs.input_digest
        || inputs
            .selected_services
            .iter()
            .any(|id| !inputs.inputs.services.contains_key(id))
    {
        return Err(invalid("source input contract is invalid"));
    }
    validate_keyed_inputs(&inputs.inputs)
}

fn validate_repository_inputs(
    inputs: &RepositoryInputs,
    expected_digest: Option<&str>,
) -> Result<String, ClewError> {
    if inputs.update_state.schema != UPDATE_STATE_SCHEMA {
        return Err(invalid("declaration input contract is invalid"));
    }
    let actual_digest = digest(inputs)?;
    if expected_digest.is_some_and(|expected| expected != actual_digest) {
        return Err(invalid("declaration input contract is invalid"));
    }
    validate_keyed_inputs(inputs)?;
    Ok(actual_digest)
}

fn store_manifest(
    repo: &Repository,
    schema: &str,
    inputs: &RepositoryInputs,
    input_digest: &str,
    selected_services: BTreeSet<String>,
) -> Result<cache::ObjectRef, ClewError> {
    let stored = Manifest {
        schema: schema.into(),
        input_digest: input_digest.into(),
        manifest: inputs.manifest.clone(),
        selected_services,
        services: store_map(repo, SERVICE_SCHEMA, &inputs.services)?,
        interactions: store_map(repo, INTERACTION_SCHEMA, &inputs.interactions)?,
        scenarios: store_map(repo, SCENARIO_SCHEMA, &inputs.scenarios)?,
        entities: store_map(repo, ENTITY_SCHEMA, &inputs.entities)?,
        notes: store_notes(repo, &inputs.notes)?,
        evidence_expectations: store_map(repo, EXPECTATION_SCHEMA, &inputs.evidence_expectations)?,
        update_policies: store_map(repo, UPDATE_POLICY_SCHEMA, &inputs.update_policies)?,
        update_state: UpdateStateReferences {
            schema: inputs.update_state.schema.clone(),
            targets: store_map(repo, UPDATE_EVENT_SCHEMA, &inputs.update_state.targets)?,
        },
    };
    cache::put_json(repo, schema, &stored)
}

fn load_manifest(
    repo: &Repository,
    reference: &cache::ObjectRef,
    schema: &str,
) -> Result<Manifest, ClewError> {
    check_reference(reference, schema)?;
    let manifest: Manifest = cache::get_json(repo, reference, check::PORTABLE_CACHE_MAX_BYTES)?
        .ok_or_else(|| state_corrupt("source input manifest is missing"))?;
    if manifest.schema != schema || manifest.update_state.schema != UPDATE_STATE_SCHEMA {
        return Err(invalid("unsupported source input manifest schema"));
    }
    Ok(manifest)
}

fn hydrate_manifest(repo: &Repository, manifest: &Manifest) -> Result<RepositoryInputs, ClewError> {
    Ok(RepositoryInputs {
        manifest: manifest.manifest.clone(),
        services: load_map(repo, SERVICE_SCHEMA, &manifest.services)?,
        interactions: load_map(repo, INTERACTION_SCHEMA, &manifest.interactions)?,
        scenarios: load_map(repo, SCENARIO_SCHEMA, &manifest.scenarios)?,
        entities: load_map(repo, ENTITY_SCHEMA, &manifest.entities)?,
        notes: load_notes(repo, &manifest.notes)?,
        evidence_expectations: load_map(repo, EXPECTATION_SCHEMA, &manifest.evidence_expectations)?,
        update_policies: load_map(repo, UPDATE_POLICY_SCHEMA, &manifest.update_policies)?,
        update_state: super::updates::State {
            schema: manifest.update_state.schema.clone(),
            targets: load_map(repo, UPDATE_EVENT_SCHEMA, &manifest.update_state.targets)?,
        },
    })
}

pub(super) fn store(
    repo: &Repository,
    inputs: &check::SourceInputs,
) -> Result<cache::ObjectRef, ClewError> {
    validate(inputs)?;
    store_manifest(
        repo,
        MANIFEST_SCHEMA,
        &inputs.inputs,
        &inputs.input_digest,
        inputs.selected_services.clone(),
    )
}

pub(super) fn load(
    repo: &Repository,
    reference: &cache::ObjectRef,
) -> Result<check::SourceInputs, ClewError> {
    let manifest = load_manifest(repo, reference, MANIFEST_SCHEMA)?;
    let inputs = hydrate_manifest(repo, &manifest)?;
    let result = check::SourceInputs {
        schema: check::SOURCE_INPUTS_SCHEMA.into(),
        input_digest: manifest.input_digest,
        inputs,
        selected_services: manifest.selected_services,
    };
    validate(&result)?;
    Ok(result)
}

pub(super) fn store_declarations(
    repo: &Repository,
    inputs: &RepositoryInputs,
    expected_digest: &str,
) -> Result<cache::ObjectRef, ClewError> {
    validate_repository_inputs(inputs, Some(expected_digest))?;
    store_manifest(
        repo,
        DECLARATION_MANIFEST_SCHEMA,
        inputs,
        expected_digest,
        BTreeSet::new(),
    )
}

pub(super) fn load_declarations(
    repo: &Repository,
    reference: &cache::ObjectRef,
) -> Result<(RepositoryInputs, String), ClewError> {
    let manifest = load_manifest(repo, reference, DECLARATION_MANIFEST_SCHEMA)?;
    if !manifest.selected_services.is_empty() {
        return Err(invalid(
            "declaration input manifest cannot carry selected source services",
        ));
    }
    let inputs = hydrate_manifest(repo, &manifest)?;
    validate_repository_inputs(&inputs, Some(&manifest.input_digest))
        .map_err(|_| state_corrupt("declaration input manifest digest or key validation failed"))?;
    Ok((inputs, manifest.input_digest))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::documentation::model::Manifest as DocumentationManifest;
    use std::fs;

    fn repository() -> (tempfile::TempDir, Repository) {
        let temporary = tempfile::tempdir().unwrap();
        let repo = Repository {
            root: temporary.path().to_path_buf(),
            manifest: DocumentationManifest {
                schema: "codeclew-documentation/1.0".into(),
                title: "Architecture".into(),
            },
        };
        (temporary, repo)
    }

    fn source_inputs(note_count: usize) -> check::SourceInputs {
        let mut notes: BTreeMap<String, Value> = (0..note_count)
            .map(|index| {
                let id = format!("note-{index:02}");
                (
                    id.clone(),
                    serde_json::json!({
                        "association": {
                            "schema": "codeclew-documentation-note-association/1.0",
                            "id": id,
                            "title": "A note",
                            "service": "orders",
                            "path": "notes/orders.md",
                            "targets": ["service:orders"],
                            "classification": "fact",
                            "period": "2026",
                            "tags": [],
                            "metadata": {}
                        },
                        "original": {
                            "status": "CAPTURED",
                            "digest": "sha256:original",
                            "text": format!("{id}:{}", "x".repeat(16 * 1024))
                        },
                        "associationDigest": "sha256:association",
                        "authority": "HUMAN_OR_IMPORTED_UNVERIFIED"
                    }),
                )
            })
            .collect();
        for note in notes.values_mut() {
            note["associationDigest"] = digest(&note["association"]).unwrap().into();
            note["original"]["digest"] = digest(&note["original"]["text"]).unwrap().into();
        }
        let inputs = RepositoryInputs {
            manifest: DocumentationManifest {
                schema: "codeclew-documentation/1.0".into(),
                title: "Architecture".into(),
            },
            services: BTreeMap::new(),
            interactions: BTreeMap::new(),
            scenarios: BTreeMap::new(),
            entities: BTreeMap::new(),
            notes,
            evidence_expectations: BTreeMap::new(),
            update_policies: BTreeMap::new(),
            update_state: super::super::updates::State {
                schema: UPDATE_STATE_SCHEMA.into(),
                targets: BTreeMap::new(),
            },
        };
        check::SourceInputs {
            schema: check::SOURCE_INPUTS_SCHEMA.into(),
            input_digest: digest(&inputs).unwrap(),
            inputs,
            selected_services: BTreeSet::new(),
        }
    }

    fn object_bytes(repo: &Repository) -> u64 {
        cache::owned_digests(repo, 1024)
            .unwrap()
            .into_iter()
            .map(|digest| {
                fs::metadata(
                    repo.root
                        .join(cache::OBJECT_ROOT)
                        .join(digest)
                        .join("object.json"),
                )
                .unwrap()
                .len()
            })
            .sum()
    }

    fn payload_allocated_bytes(repo: &Repository) -> u64 {
        use std::os::unix::fs::MetadataExt;
        cache::owned_digests(repo, 1024)
            .unwrap()
            .iter()
            .map(|digest| {
                fs::metadata(
                    repo.root
                        .join(cache::OBJECT_ROOT)
                        .join(digest)
                        .join("object.json"),
                )
                .unwrap()
                .blocks()
                    * 512
            })
            .sum()
    }

    #[test]
    fn note_change_reuses_original_objects_and_round_trips_exactly() {
        let (_temporary, repo) = repository();
        let source_a = source_inputs(32);
        let handle_a = store(&repo, &source_a).unwrap();
        let mut digests_a = cache::owned_digests(&repo, 1024).unwrap();
        digests_a.sort();
        let bytes_a = object_bytes(&repo);
        let allocated_a = payload_allocated_bytes(&repo);

        let mut source_b = source_a.clone();
        source_b.inputs.notes.get_mut("note-07").unwrap()["association"]["title"] =
            "A changed note".into();
        let note = source_b.inputs.notes.get_mut("note-07").unwrap();
        note["associationDigest"] = digest(&note["association"]).unwrap().into();
        source_b.input_digest = digest(&source_b.inputs).unwrap();
        let handle_b = store(&repo, &source_b).unwrap();
        let mut digests_b = cache::owned_digests(&repo, 1024).unwrap();
        digests_b.sort();
        let manifest_a: Manifest =
            cache::get_json(&repo, &handle_a, check::PORTABLE_CACHE_MAX_BYTES)
                .unwrap()
                .unwrap();
        let manifest_b: Manifest =
            cache::get_json(&repo, &handle_b, check::PORTABLE_CACHE_MAX_BYTES)
                .unwrap()
                .unwrap();
        let loaded_b = load(&repo, &handle_b).unwrap();
        assert_eq!(
            crate::canonical::bytes(&loaded_b).unwrap(),
            crate::canonical::bytes(&source_b).unwrap()
        );
        assert_eq!(
            load(&repo, &handle_a).unwrap().inputs.notes["note-07"]["original"],
            loaded_b.inputs.notes["note-07"]["original"]
        );
        assert_eq!(
            manifest_a.notes["note-07"].original,
            manifest_b.notes["note-07"].original
        );
        for id in source_a.inputs.notes.keys().filter(|id| *id != "note-07") {
            assert_eq!(manifest_a.notes[id].metadata, manifest_b.notes[id].metadata);
            assert_eq!(manifest_a.notes[id].original, manifest_b.notes[id].original);
        }
        assert_eq!(digests_b.len() - digests_a.len(), 2);
        assert!(object_bytes(&repo) - bytes_a < 32 * 16 * 1024);

        let bytes_b = object_bytes(&repo);
        let allocated_b = payload_allocated_bytes(&repo);
        let handle_b_again = store(&repo, &source_b).unwrap();
        assert_eq!(handle_b_again, handle_b);
        let mut repeat_digests = cache::owned_digests(&repo, 1024).unwrap();
        repeat_digests.sort();
        assert_eq!(repeat_digests, digests_b);
        assert_eq!(object_bytes(&repo), bytes_b);

        let mut source_c = source_b.clone();
        let note = source_c.inputs.notes.get_mut("note-07").unwrap();
        let text = format!("{}!", note["original"]["text"].as_str().unwrap());
        note["original"]["digest"] = digest(&text).unwrap().into();
        note["original"]["text"] = text.into();
        source_c.input_digest = digest(&source_c.inputs).unwrap();
        let handle_c = store(&repo, &source_c).unwrap();
        let manifest_c: Manifest =
            cache::get_json(&repo, &handle_c, check::PORTABLE_CACHE_MAX_BYTES)
                .unwrap()
                .unwrap();
        let after_c = cache::inventory(&repo).unwrap();
        assert_eq!(after_c.object_count - digests_b.len(), 2);
        assert_eq!(
            manifest_c.notes["note-07"].metadata,
            manifest_b.notes["note-07"].metadata
        );
        for id in source_b.inputs.notes.keys().filter(|id| *id != "note-07") {
            assert_eq!(manifest_c.notes[id].original, manifest_b.notes[id].original);
            assert_eq!(manifest_c.notes[id].metadata, manifest_b.notes[id].metadata);
        }
        assert_eq!(
            crate::canonical::bytes(&load(&repo, &handle_c).unwrap()).unwrap(),
            crate::canonical::bytes(&source_c).unwrap()
        );
        let text_delta = after_c.object_bytes - bytes_b;
        assert_eq!(
            text_delta,
            handle_c.size + manifest_c.notes["note-07"].original.size
        );
        assert_eq!(store(&repo, &source_c).unwrap(), handle_c);
        let repeated = cache::inventory(&repo).unwrap();
        assert_eq!(repeated.object_count, after_c.object_count);
        assert_eq!(repeated.object_bytes, after_c.object_bytes);
        println!(
            "SOURCE_INPUT_STORAGE {}",
            serde_json::json!({
                "notes":32,"textBytesPerNote":16384,
                "wholeInputBytes":crate::canonical::bytes(&source_c.inputs).unwrap().len(),
                "initialObjects":digests_a.len(),"initialPayloadBytes":bytes_a,
            "initialPayloadAllocatedBytes":allocated_a,
            "titleChangeNewPayloadAllocatedBytes":allocated_b-allocated_a,
            "textChangeNewPayloadAllocatedBytes":payload_allocated_bytes(&repo)-allocated_b,
                "titleChangeNewObjects":digests_b.len()-digests_a.len(),
                "titleChangeNewPayloadBytes":bytes_b-bytes_a,
                "textChangeNewObjects":after_c.object_count-digests_b.len(),
                "textChangeNewPayloadBytes":text_delta,
                "repeatNewObjects":0,"repeatNewPayloadBytes":0,
                "scope":"CAS payload files; st_blocks is not exclusive APFS space; directories/build and overlay wrappers excluded"
            })
        );
    }

    #[test]
    fn missing_or_corrupt_note_payload_is_not_replaced_by_ambient_files() {
        let (_temporary, repo) = repository();
        let source = source_inputs(1);
        let handle = store(&repo, &source).unwrap();
        let manifest: Manifest = cache::get_json(&repo, &handle, check::PORTABLE_CACHE_MAX_BYTES)
            .unwrap()
            .unwrap();
        let object = repo
            .root
            .join(cache::OBJECT_ROOT)
            .join(&manifest.notes["note-00"].original.digest)
            .join("object.json");
        let original = fs::read(&object).unwrap();
        fs::remove_file(&object).unwrap();
        assert!(
            load(&repo, &handle)
                .unwrap_err()
                .message
                .contains("note original object is missing")
        );
        fs::write(&object, vec![b'x'; original.len()]).unwrap();
        assert!(load(&repo, &handle).is_err());
    }

    #[test]
    fn declaration_codec_has_distinct_manifest_and_reuses_typed_payloads() {
        let (_temporary, repo) = repository();
        let source = source_inputs(1);
        let source_handle = store(&repo, &source).unwrap();
        let declaration_digest = digest(&source.inputs).unwrap();
        let declaration_handle =
            store_declarations(&repo, &source.inputs, &declaration_digest).unwrap();
        assert_eq!(source_handle.schema, MANIFEST_SCHEMA);
        assert_eq!(declaration_handle.schema, DECLARATION_MANIFEST_SCHEMA);
        assert_ne!(source_handle.digest, declaration_handle.digest);

        let source_manifest: Manifest =
            cache::get_json(&repo, &source_handle, check::PORTABLE_CACHE_MAX_BYTES)
                .unwrap()
                .unwrap();
        let declaration_manifest: Manifest =
            cache::get_json(&repo, &declaration_handle, check::PORTABLE_CACHE_MAX_BYTES)
                .unwrap()
                .unwrap();
        assert_eq!(
            source_manifest.notes["note-00"].metadata,
            declaration_manifest.notes["note-00"].metadata
        );
        assert_eq!(
            source_manifest.notes["note-00"].original,
            declaration_manifest.notes["note-00"].original
        );
        let (loaded, loaded_digest) = load_declarations(&repo, &declaration_handle).unwrap();
        assert_eq!(loaded_digest, declaration_digest);
        assert_eq!(
            crate::canonical::bytes(&loaded).unwrap(),
            crate::canonical::bytes(&source.inputs).unwrap()
        );
        assert!(load_declarations(&repo, &source_handle).is_err());

        let mut selected = declaration_manifest.clone();
        selected.selected_services.insert("orders".into());
        let selected_handle =
            cache::put_json(&repo, DECLARATION_MANIFEST_SCHEMA, &selected).unwrap();
        assert!(load_declarations(&repo, &selected_handle).is_err());

        let mut bad_checksum = declaration_manifest;
        bad_checksum.input_digest = format!("sha256:{}", "0".repeat(64));
        let bad_checksum_handle =
            cache::put_json(&repo, DECLARATION_MANIFEST_SCHEMA, &bad_checksum).unwrap();
        assert!(load_declarations(&repo, &bad_checksum_handle).is_err());
    }

    #[test]
    fn missing_or_corrupt_source_input_manifest_fails_closed() {
        let (_temporary, repo) = repository();
        let missing = cache::ObjectRef::new(
            MANIFEST_SCHEMA.into(),
            format!("sha256:{}", "f".repeat(64)),
            1,
        );
        assert!(load(&repo, &missing).is_err());

        let source = source_inputs(0);
        let handle = store(&repo, &source).unwrap();
        let object = repo
            .root
            .join(cache::OBJECT_ROOT)
            .join(&handle.digest)
            .join("object.json");
        fs::write(object, b"corrupt").unwrap();
        assert!(load(&repo, &handle).is_err());
    }
}
