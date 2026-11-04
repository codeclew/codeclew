//! Explicit source associations to separately captured Kotlin/JVM libraries.
//! Binary ownership, declared source binding and B compiler evidence are distinct.
use super::{
    analysis, bytes, cache,
    check::Check,
    digest, invalid,
    model::*,
    store::{self, Repository},
};
use crate::error::ClewError;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

const SCHEMA: &str = "codeclew-documentation-library-binding/1.0";
const INPUT_SCHEMA: &str = "codeclew-documentation-library-binding-input/1.0";
const AUTHORITY: &str = "DECLARED_SOURCE_BINDING";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Input {
    pub schema: String,
    pub id: String,
    pub consumer: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consumer_revision: Option<String>,
    /// Empty only for SOURCE_SYNTAX; never stands for a compiler wildcard.
    pub consumer_compilation: String,
    /// Exact retained artifact row for compiler A; declared coordinates/hash for SOURCE A.
    pub artifact: Value,
    pub library: LibrarySelection,
    pub authority: String,
    pub rationale: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LibrarySelection {
    pub service: String,
    pub repository_id: String,
    pub repository: String,
    pub revision: String,
    pub compilation: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Binding {
    pub schema: String,
    pub input: Input,
    /// Foreign identity is not a local Check object reference. Only capture below
    /// references locally imported immutable objects and participates in closure.
    pub origin_snapshot_digest: String,
    pub origin_snapshot_bytes: u64,
    pub service: Service,
    pub capture: cache::CaptureManifest,
}
fn coordinates(value: &Value) -> Result<(), ClewError> {
    if !["group", "name", "version", "extension"].iter().all(|k| {
        value[k]
            .as_str()
            .is_some_and(|s| !s.is_empty() && s.len() <= 512 && !s.chars().any(char::is_control))
    }) || !value["classifier"].is_null()
        && !value["classifier"].as_str().is_some_and(|s| s.len() <= 512)
    {
        return Err(invalid(
            "LIBRARY_COORDINATES_INVALID: specify group, name, version, classifier and extension",
        ));
    }
    Ok(())
}
fn validate_input(v: &Input) -> Result<(), ClewError> {
    let source = v.consumer_compilation.is_empty();
    if v.schema != INPUT_SCHEMA
        || !store::valid_id(&v.id)
        || !store::valid_id(&v.consumer)
        || !store::valid_id(&v.library.service)
        || !store::valid_id(&v.library.repository_id)
        || v.authority != AUTHORITY
        || v.rationale.trim().is_empty()
        || v.rationale.len() > 4096
        || v.consumer_compilation.len() > 512
        || v.library.compilation.is_empty()
        || v.library.compilation.len() > 512
        || v.library.revision.len() != 40
        || !v.library.revision.bytes().all(|b| b.is_ascii_hexdigit())
        || v.consumer_revision
            .as_ref()
            .is_some_and(|r| r.len() != 40 || !r.bytes().all(|b| b.is_ascii_hexdigit()))
        || (!source || !v.artifact["contentDigest"].is_null())
            && !super::library_artifacts::is_digest(
                v.artifact["contentDigest"].as_str().unwrap_or(""),
            )
        || (!source || !v.artifact["variant"].is_null()) && !v.artifact["variant"].is_object()
        || (!source || !v.artifact["size"].is_null()) && v.artifact["size"].as_u64().is_none()
    {
        return Err(invalid(
            "LIBRARY_BINDING_INVALID: explicit pinned artifact, repository, revision, compilation and declared rationale required",
        ));
    }
    super::source_links::repository(&v.library.repository).map_err(invalid)?;
    coordinates(&v.artifact["coordinates"])
}
pub(super) fn validate(v: &Binding) -> Result<(), ClewError> {
    validate_input(&v.input)?;
    store::validate_service(&v.service)?;
    cache::validate_capture(&v.capture)?;
    let s = &v.input.library;
    if v.schema != SCHEMA
        || !super::library_artifacts::is_digest(&v.origin_snapshot_digest)
        || v.origin_snapshot_bytes == 0
        || v.origin_snapshot_bytes > super::check::PORTABLE_CACHE_MAX_BYTES
        || v.service.id != s.service
        || v.service.repository_id != s.repository_id
        || v.service.repository != s.repository
        || v.capture.service != s.service
        || v.capture.revision != s.revision
        || v.capture.service_digest != digest(&v.service)?
        || v.capture.extractor != EXTRACTOR
        || v.service.profile != "kotlin-jvm-gradle-analysis"
        || v.service.language != "kotlin"
        || !v.service.compilations.contains(&s.compilation)
    {
        return Err(invalid(
            "LIBRARY_CAPTURE_IDENTITY_MISMATCH: use the selected compiler snapshot/repository/revision/compilation",
        ));
    }
    Ok(())
}
pub fn records(repo: &Repository) -> Result<BTreeMap<String, Binding>, ClewError> {
    let records: BTreeMap<String, Binding> = repo.records(".codeclew/library-bindings", "json")?;
    for (id, v) in &records {
        validate(v)?;
        if id != &v.input.id {
            return Err(invalid("library binding filename differs from ID"));
        }
    }
    Ok(records)
}
pub(super) fn validate_inputs(inputs: &super::store::RepositoryInputs) -> Result<(), ClewError> {
    if inputs.library_bindings.len() > 64 {
        return Err(invalid("library binding budget exceeded (64)"));
    }
    let mut admitted = BTreeMap::new();
    for (id, b) in &inputs.library_bindings {
        validate(b)?;
        if id != &b.input.id || !inputs.services.contains_key(&b.input.consumer) {
            return Err(invalid("library binding must name a registered consumer"));
        }
        let key = (
            &b.input.consumer,
            &b.input.consumer_compilation,
            digest(&b.input.artifact)?,
        );
        if admitted.insert(key, id).is_some() {
            return Err(invalid(
                "LIBRARY_ASSOCIATION_AMBIGUOUS: one consumer artifact cannot select multiple library captures",
            ));
        }
        let a = &inputs.services[&b.input.consumer];
        if a.language != "kotlin" {
            return Err(invalid(
                "LIBRARY_CONSUMER_LANGUAGE_UNSUPPORTED: this library association path supports Kotlin consumers",
            ));
        }
        if a.repository_id == b.service.repository_id || a.repository == b.service.repository {
            return Err(invalid(
                "library source must have an independent repository identity",
            ));
        }
    }
    Ok(())
}

/// Validate the real producer artifact and B's declared publication separately.
/// This establishes an admissible *declared* association, never binary equivalence.
pub(super) fn admit(
    input: &Input,
    a: &ServiceEvidence,
    b: &ServiceEvidence,
) -> Result<(), ClewError> {
    validate_input(input)?;
    analysis::verify_evidence(b)?;
    if b.extractor != EXTRACTOR {
        return Err(invalid(
            "LIBRARY_COMPILER_CAPTURE_REQUIRED: source syntax is not a compiler B capture",
        ));
    }
    let c = &input.artifact["coordinates"];
    let publications = b
        .observations
        .values()
        .filter(|o| {
            o.kind == "LIBRARY_PUBLICATION" && o.normalized["scope"] == input.library.compilation
        })
        .collect::<Vec<_>>();
    if !publications.iter().any(|o| {
        ["group", "name", "version"]
            .iter()
            .all(|k| o.normalized[k] == c[k])
    }) {
        return Err(invalid(
            "LIBRARY_PUBLICATION_MISMATCH: artifact coordinates/version do not match a publication declaration in the selected B compilation; capture the corresponding version",
        ));
    }
    if a.extractor == SOURCE_EXTRACTOR {
        if input.consumer_revision.as_deref() != Some(a.revision.as_str()) {
            return Err(invalid(
                "LIBRARY_SOURCE_CONSUMER_REVISION_MISMATCH: explicitly review and pin the dependency declaration to this SOURCE capture revision; no A build resolution is implied",
            ));
        }
        if !input.consumer_compilation.is_empty() {
            return Err(invalid(
                "SOURCE library binding cannot claim a compiler compilation",
            ));
        }
    } else {
        if input.consumer_compilation.is_empty()
            || !a.observations.values().any(|o| {
                o.kind == "DEPENDENCY_ARTIFACT"
                    && o.normalized["scope"] == input.consumer_compilation
                    && {
                        let mut value = o.normalized.clone();
                        value.as_object_mut().unwrap().remove("scope");
                        value.as_object_mut().unwrap().remove("evidenceBinding");
                        value == input.artifact
                    }
            })
        {
            return Err(invalid(
                "LIBRARY_ARTIFACT_MISMATCH: select the exact resolved artifact row/hash/variant from A's retained compilation; a declared version is insufficient",
            ));
        }
    }
    if b.revision != input.library.revision || b.service != input.library.service {
        return Err(invalid(
            "LIBRARY_SOURCE_IDENTITY_MISMATCH: select the exact B capture",
        ));
    }
    Ok(())
}

/// Import one already captured library, with no analyzer or checkout access.
/// Installation into current inputs is a separate operation so validation can
/// complete before a registry mutation and failed association leaves A readable.
pub(super) fn import(
    repo: &Repository,
    checked: &Check,
    library_repo: &Repository,
    library_snapshot: &str,
    input: Input,
) -> Result<Binding, ClewError> {
    validate_input(&input)?;
    let foreign = Check::load_snapshot(library_repo, library_snapshot)?;
    let service = foreign
        .source_inputs
        .as_ref()
        .and_then(|s| s.inputs.services.get(&input.library.service))
        .ok_or_else(|| {
            invalid("LIBRARY_SOURCE_DECLARATION_MISSING: select B's original compiler capture")
        })?
        .clone();
    let evidence = foreign
        .services
        .get(&input.library.service)
        .ok_or_else(|| {
            invalid("LIBRARY_CAPTURE_MISSING: explicitly capture B before attachment")
        })?;
    let a = checked
        .services
        .get(&input.consumer)
        .ok_or_else(|| invalid("LIBRARY_CONSUMER_CAPTURE_MISSING: select a retained A capture"))?;
    admit(&input, a, evidence)?;
    if foreign.composition.is_some()
        || evidence
            .sources
            .values()
            .any(|s| s.revision != evidence.revision || s.library_origin.is_some())
    {
        return Err(invalid(
            "LIBRARY_ORIGINAL_CAPTURE_REQUIRED: select B's independent original compiler capture",
        ));
    }
    let (origin_digest, size) = library_snapshot
        .rsplit_once('/')
        .ok_or_else(|| invalid("invalid B snapshot identity"))?;
    let mut capture = cache::store_capture(repo, evidence)?;
    let original = Check::load_snapshot_manifest(library_repo, library_snapshot)?;
    let original = original
        .service_manifests
        .get(&input.library.service)
        .ok_or_else(|| invalid("original B capture manifest missing"))?;
    capture.cacheability = original.cacheability.clone();
    capture.reason = original.reason.clone();
    let binding = Binding {
        schema: SCHEMA.into(),
        input,
        origin_snapshot_digest: origin_digest.into(),
        origin_snapshot_bytes: size
            .parse()
            .map_err(|_| invalid("invalid B snapshot size"))?,
        service,
        capture,
    };
    validate(&binding)?;
    Ok(binding)
}

/// Public artifact rows are bounded provenance, not source-to-binary claims.
pub(super) fn dependency_rows(e: &ServiceEvidence) -> Vec<Value> {
    e.observations.values().filter(|o| matches!(o.kind.as_str(),"DEPENDENCY_ARTIFACT"|"LIBRARY_PUBLICATION"))
        .map(|o|{
            let artifact=if o.kind=="DEPENDENCY_ARTIFACT" {
                let mut artifact=o.normalized.clone();
                artifact.as_object_mut().unwrap().remove("scope");
                artifact.as_object_mut().unwrap().remove("evidenceBinding");
                artifact
            } else {Value::Null};
            json!({"id":o.id,"kind":o.kind,"compilation":o.normalized["scope"],"artifact":artifact,"record":o.normalized})
        }).collect()
}

#[derive(Debug, clap::Subcommand)]
pub enum Command {
    /// Inspect actual resolved artifacts or publication declarations in a retained snapshot.
    Inspect {
        #[arg(long)]
        root: std::path::PathBuf,
        #[arg(long)]
        snapshot: String,
        #[arg(long)]
        service: String,
    },
    /// Import an existing independent compiler capture and declare its artifact association.
    /// Recompose the returned original capture to activate changed inputs, without capture.
    Attach {
        #[arg(long)]
        root: std::path::PathBuf,
        #[arg(long)]
        snapshot: String,
        #[arg(long)]
        library_root: std::path::PathBuf,
        #[arg(long)]
        library_snapshot: String,
        #[arg(long)]
        input: std::path::PathBuf,
        #[arg(long)]
        expected_input_digest: String,
    },
    /// Remove a current source association. Historical snapshots keep their closure.
    Detach {
        #[arg(long)]
        root: std::path::PathBuf,
        #[arg(long)]
        id: String,
        #[arg(long)]
        expected_input_digest: String,
    },
    /// List declared library associations; never opens B's checkout or runs its build.
    List {
        #[arg(long)]
        root: std::path::PathBuf,
    },
}
pub fn run(command: Command) -> Result<Value, ClewError> {
    match command {
        Command::Inspect {
            root,
            snapshot,
            service,
        } => {
            let repo = Repository::open(&root)?;
            let checked = Check::load_snapshot(&repo, &snapshot)?;
            let evidence = checked
                .services
                .get(&service)
                .ok_or_else(|| invalid("service absent from selected snapshot"))?;
            let declaration = checked
                .source_inputs
                .as_ref()
                .and_then(|i| i.inputs.services.get(&service));
            Ok(
                json!({"schema":"codeclew-documentation-library-inspection/1.0","snapshot":snapshot,"service":declaration,"revision":evidence.revision,"rows":dependency_rows(evidence),"sourceToBinaryEquivalence":"NOT_ESTABLISHED"}),
            )
        }
        Command::List { root } => {
            let repo = Repository::open(&root)?;
            Ok(
                json!({"bindings":records(&repo)?,"inputDigest":repo.input_digest()?,"authority":AUTHORITY}),
            )
        }
        Command::Attach {
            root,
            snapshot,
            library_root,
            library_snapshot,
            input,
            expected_input_digest,
        } => {
            let repo = Repository::open(&root)?;
            let library_repo = Repository::open(&library_root)?;
            if repo.root == library_repo.root {
                return Err(invalid(
                    "library capture must come from a separate documentation root",
                ));
            }
            let (checked, _) = Check::retained(&repo, Some(&snapshot), &Default::default())?;
            let input: Input = store::read(&input, 1024 * 1024)?;
            let binding = import(&repo, &checked, &library_repo, &library_snapshot, input)?;
            let mut inputs = repo.inputs()?;
            inputs
                .library_bindings
                .insert(binding.input.id.clone(), binding.clone());
            validate_inputs(&inputs)?;
            let path = format!(".codeclew/library-bindings/{}.json", binding.input.id);
            let _lock = repo.lock()?;
            if repo.input_digest()? != expected_input_digest {
                return Err(crate::error::ClewError::new(
                    crate::error::ErrorCode::WwConflict,
                    "documentation inputs changed; reload current inputDigest",
                ));
            }
            repo.atomic(&path, &bytes(&binding)?)?;
            let parent = checked
                .composition
                .as_ref()
                .map(|c| c.parent.as_str())
                .unwrap_or(&snapshot);
            Ok(
                json!({"status":"DECLARED_SOURCE_BINDING_REGISTERED","id":binding.input.id,"inputDigest":repo.input_digest()?,"originalCapture":parent,"sourceToBinaryEquivalence":"NOT_ESTABLISHED","sourceCapture":"NOT_RUN","nextAction":"Run docs recompose --snapshot ORIGINAL_CAPTURE; use its returned snapshot for Work, threads and render."}),
            )
        }
        Command::Detach {
            root,
            id,
            expected_input_digest,
        } => {
            if !store::valid_id(&id) {
                return Err(invalid("invalid library binding ID"));
            }
            let repo = Repository::open(&root)?;
            let _lock = repo.lock()?;
            if repo.input_digest()? != expected_input_digest {
                return Err(crate::error::ClewError::new(
                    crate::error::ErrorCode::WwConflict,
                    "documentation inputs changed; reload current inputDigest",
                ));
            }
            if !records(&repo)?.contains_key(&id) {
                return Err(invalid("library binding not registered"));
            }
            std::fs::remove_file(repo.path(&format!(".codeclew/library-bindings/{id}.json"))?)
                .map_err(super::io_error)?;
            Ok(
                json!({"status":"DETACHED","id":id,"inputDigest":repo.input_digest()?,"historicalEvidence":"PRESERVED","sourceCapture":"NOT_RUN","nextAction":"Recompose the original capture and select its returned snapshot; previous joined snapshots remain historical."}),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn setup() -> (Input, ServiceEvidence, ServiceEvidence) {
        let hash = format!("sha256:{}", "a".repeat(64));
        let artifact = json!({"id":format!("classpath:0:{hash}"),"classpathIndex":0,"fileName":"library.jar","contentDigest":hash,"size":100,"coordinates":{"group":"owned","name":"library","version":"1.0.0","extension":"jar","classifier":null,"scope":"compileClasspath"},"variant":{"org.gradle.usage":"java-api"},"authority":"GRADLE_RESOLVED_COMPONENT_ARTIFACT","classpathAuthority":"KOTLIN_TASK_LIBRARIES"});
        let input=Input{schema:INPUT_SCHEMA.into(),id:"owned-library".into(),consumer:"application".into(),consumer_revision:None,consumer_compilation:":/main".into(),artifact:artifact.clone(),library:LibrarySelection{service:"library".into(),repository_id:"owned-library".into(),repository:"https://github.com/example/owned-library.git".into(),revision:"b".repeat(40),compilation:":/main".into()},authority:AUTHORITY.into(),rationale:"Owner explicitly associates this compiler source capture; no binary equivalence attestation.".into()};
        let mut a = ServiceEvidence {
            schema: "codeclew-documentation-service-evidence/1.0".into(),
            service: "application".into(),
            revision: "a".repeat(40),
            service_digest: "input".into(),
            extractor: EXTRACTOR.into(),
            runtime_mode: "RELEASE".into(),
            coverage: "PARTIAL".into(),
            boundaries: vec!["ORIGINAL_PROVIDER_LIMIT".into()],
            entrypoints: vec![],
            observations: BTreeMap::new(),
            sources: BTreeMap::new(),
            contracts: BTreeMap::new(),
        };
        let mut row = artifact;
        row["scope"] = json!(":/main");
        row["evidenceBinding"] = json!(hash);
        a.observations.insert(
            "artifact".into(),
            Observation {
                id: "artifact".into(),
                kind: "DEPENDENCY_ARTIFACT".into(),
                service: a.service.clone(),
                symbol: "artifact".into(),
                digest: digest(&row).unwrap(),
                normalized: row,
                source_ids: vec![],
            },
        );
        let mut b = a.clone();
        b.service = "library".into();
        b.revision = "b".repeat(40);
        b.observations.clear();
        let row = json!({"group":"owned","name":"library","version":"1.0.0","authority":"GRADLE_PUBLICATION_DECLARATION","scope":":/main"});
        b.observations.insert(
            "publication".into(),
            Observation {
                id: "publication".into(),
                kind: "LIBRARY_PUBLICATION".into(),
                service: b.service.clone(),
                symbol: "publication".into(),
                digest: digest(&row).unwrap(),
                normalized: row,
                source_ids: vec![],
            },
        );
        (input, a, b)
    }
    #[test]
    fn declared_association_requires_exact_compiler_artifact_and_publication() {
        let (input, a, b) = setup();
        admit(&input, &a, &b).unwrap();
        assert_eq!(dependency_rows(&a)[0]["artifact"], input.artifact);
        for field in ["contentDigest", "variant", "coordinates", "id", "size"] {
            let mut bad = input.clone();
            bad.artifact[field] = match field {
                "contentDigest" => json!(format!("sha256:{}", "c".repeat(64))),
                "variant" => json!({"org.gradle.usage":"java-runtime"}),
                "coordinates" => {
                    let mut v = bad.artifact[field].clone();
                    v["version"] = json!("2.0.0");
                    v
                }
                "size" => json!(101),
                _ => json!("classpath:1:other"),
            };
            assert!(admit(&bad, &a, &b).is_err(), "{field}");
        }
        let mut wrong_scope = input.clone();
        wrong_scope.consumer_compilation = ":other/main".into();
        assert!(admit(&wrong_scope, &a, &b).is_err());
        let mut wrong_revision = input.clone();
        wrong_revision.library.revision = "c".repeat(40);
        assert!(admit(&wrong_revision, &a, &b).is_err());
        assert_eq!(a.boundaries, ["ORIGINAL_PROVIDER_LIMIT"]);
    }
    #[test]
    fn source_association_remains_declared_and_same_version_source_is_not_equivalence() {
        let (mut input, mut a, b) = setup();
        a.extractor = SOURCE_EXTRACTOR.into();
        a.coverage = "SYNTAX".into();
        a.observations.clear();
        assert!(admit(&input, &a, &b).is_err());
        input.consumer_compilation.clear();
        input.consumer_revision = Some(a.revision.clone());
        input.artifact = json!({"coordinates":input.artifact["coordinates"]});
        admit(&input, &a, &b).unwrap();
        let mut stale = input.clone();
        stale.consumer_revision = Some("d".repeat(40));
        assert!(admit(&stale, &a, &b).is_err());
        let mut other = b.clone();
        other.revision = "c".repeat(40);
        input.library.revision = other.revision.clone();
        admit(&input, &a, &other).unwrap();
        assert_eq!(input.authority, "DECLARED_SOURCE_BINDING");
        input.authority = "SOURCE_BINARY_EQUIVALENT".into();
        assert!(admit(&input, &a, &other).is_err());
    }
}
