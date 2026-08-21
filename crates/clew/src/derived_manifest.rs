use crate::adapter_v2::{CompilationDescriptor, PROVIDER_PROTOCOL, ProviderModel};
use crate::canonical;
use crate::cas::{CasObject, CasStore};
use crate::error::{ClewError, ErrorCode};
use crate::repository_snapshot::SNAPSHOT_SCHEMA;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const DERIVED_MANIFEST_SCHEMA: &str = "codeclew-derived-analysis-input-manifest/2.0";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DerivedAnalysisInputManifest {
    pub schema: String,
    pub manifest_id: String,
    pub repository_snapshot: CasObject,
    pub provider_models: Vec<ProviderModel>,
    pub sealed_inputs: Vec<CasObject>,
}

impl DerivedAnalysisInputManifest {
    pub fn create(
        store: &CasStore,
        repository_snapshot: CasObject,
        mut provider_models: Vec<ProviderModel>,
    ) -> Result<(Self, CasObject), ClewError> {
        if repository_snapshot.object_schema != SNAPSHOT_SCHEMA {
            return Err(invalid(
                "derived manifest requires a repository snapshot v2 object",
            ));
        }
        canonicalize_models(&mut provider_models);
        validate_models(&provider_models)?;
        let sealed_inputs = collect_inputs(&repository_snapshot, &provider_models)?;
        verify_objects(store, &sealed_inputs)?;
        let mut manifest = Self {
            schema: DERIVED_MANIFEST_SCHEMA.into(),
            manifest_id: String::new(),
            repository_snapshot,
            provider_models,
            sealed_inputs,
        };
        manifest.manifest_id = canonical::hash(&manifest).map_err(internal)?;
        let bytes = canonical::bytes(&manifest).map_err(internal)?;
        let object = store.put(DERIVED_MANIFEST_SCHEMA, &bytes)?;
        Ok((manifest, object))
    }

    pub fn verify(&self, store: &CasStore) -> Result<(), ClewError> {
        let mut unsigned = self.clone();
        unsigned.manifest_id.clear();
        if self.schema != DERIVED_MANIFEST_SCHEMA
            || self.repository_snapshot.object_schema != SNAPSHOT_SCHEMA
            || self.manifest_id != canonical::hash(&unsigned).map_err(internal)?
        {
            return Err(corrupt(
                "derived analysis input manifest identity is invalid",
            ));
        }
        let mut canonical_models = self.provider_models.clone();
        canonicalize_models(&mut canonical_models);
        if canonical_models != self.provider_models {
            return Err(corrupt(
                "derived analysis provider models are not canonical",
            ));
        }
        validate_models(&self.provider_models)?;
        if collect_inputs(&self.repository_snapshot, &self.provider_models)? != self.sealed_inputs {
            return Err(corrupt("derived analysis sealed input set is incomplete"));
        }
        verify_objects(store, &self.sealed_inputs)
    }
}

fn canonicalize_models(models: &mut [ProviderModel]) {
    for provider in models.iter_mut() {
        provider
            .build_model
            .compilations
            .sort_by(|left, right| left.compilation_id.cmp(&right.compilation_id));
        for compilation in &mut provider.build_model.compilations {
            compilation
                .source_roots
                .sort_by(|left, right| left.logical_name.cmp(&right.logical_name));
            compilation
                .generated_source_roots
                .sort_by(|left, right| left.logical_name.cmp(&right.logical_name));
            compilation.classpath.sort_by(object_order);
            compilation.plugins.sort_by(object_order);
            compilation.dependency_compilation_ids.sort();
            compilation.operations.sort_by(|left, right| {
                left.operation_uri
                    .as_str()
                    .cmp(right.operation_uri.as_str())
            });
        }
    }
    models.sort_by(|left, right| left.handshake.provider_id.cmp(&right.handshake.provider_id));
}

fn validate_models(models: &[ProviderModel]) -> Result<(), ClewError> {
    if models.is_empty() || models.len() > 4096 {
        return Err(invalid("derived analysis provider model set is invalid"));
    }
    let mut providers = BTreeSet::new();
    let mut compilations = BTreeMap::new();
    for provider in models {
        if provider.handshake.protocol != PROVIDER_PROTOCOL
            || !safe_id(&provider.handshake.provider_id)
            || !canonical_digest(&provider.handshake.provider_digest)
            || provider.handshake.build_system_uris.is_empty()
            || !provider
                .handshake
                .build_system_uris
                .windows(2)
                .all(|pair| pair[0] < pair[1])
            || provider.handshake.build_system_uris.iter().any(|uri| {
                !uri.contains(':')
                    || uri.len() > 256
                    || uri
                        .bytes()
                        .any(|byte| byte.is_ascii_control() || byte == b' ')
            })
            || provider.handshake.provider_id != provider.build_model.provider_id
            || !providers.insert(&provider.handshake.provider_id)
        {
            return Err(invalid(
                "derived analysis provider identity is duplicated or mismatched",
            ));
        }
        for compilation in &provider.build_model.compilations {
            compilation.validate()?;
            if compilations
                .insert(compilation.compilation_id.clone(), compilation)
                .is_some()
            {
                return Err(invalid(
                    "compilation identity is duplicated across providers",
                ));
            }
        }
    }
    for compilation in compilations.values() {
        for dependency in &compilation.dependency_compilation_ids {
            if !compilations.contains_key(dependency) {
                return Err(invalid(
                    "compilation dependency is missing from the sealed model",
                ));
            }
        }
    }
    validate_acyclic(&compilations)
}

fn safe_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}

fn canonical_digest(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn validate_acyclic(
    compilations: &BTreeMap<String, &CompilationDescriptor>,
) -> Result<(), ClewError> {
    let mut incoming = compilations
        .iter()
        .map(|(id, compilation)| (id.clone(), compilation.dependency_compilation_ids.len()))
        .collect::<BTreeMap<_, _>>();
    let mut dependents = BTreeMap::<String, Vec<String>>::new();
    for (id, compilation) in compilations {
        for dependency in &compilation.dependency_compilation_ids {
            dependents
                .entry(dependency.clone())
                .or_default()
                .push(id.clone());
        }
    }
    let mut ready = incoming
        .iter()
        .filter_map(|(id, count)| (*count == 0).then_some(id.clone()))
        .collect::<BTreeSet<_>>();
    let mut visited = 0usize;
    while let Some(id) = ready.iter().next().cloned() {
        ready.remove(&id);
        visited += 1;
        for dependent in dependents.get(&id).into_iter().flatten() {
            let count = incoming
                .get_mut(dependent)
                .expect("validated dependent compilation");
            *count -= 1;
            if *count == 0 {
                ready.insert(dependent.clone());
            }
        }
    }
    if visited != compilations.len() {
        return Err(invalid("compilation dependency graph contains a cycle"));
    }
    Ok(())
}

fn collect_inputs(
    repository_snapshot: &CasObject,
    models: &[ProviderModel],
) -> Result<Vec<CasObject>, ClewError> {
    let mut objects = vec![repository_snapshot.clone()];
    for provider in models {
        objects.push(provider.build_model.model.clone());
        for compilation in &provider.build_model.compilations {
            objects.extend(
                compilation
                    .source_roots
                    .iter()
                    .chain(&compilation.generated_source_roots)
                    .map(|root| root.tree.clone()),
            );
            objects.extend(compilation.classpath.iter().cloned());
            objects.push(compilation.toolchain.clone());
            objects.extend(compilation.plugins.iter().cloned());
            objects.push(compilation.canonical_options.clone());
        }
    }
    objects.sort_by(object_order);
    let mut unique = Vec::<CasObject>::with_capacity(objects.len());
    for object in objects {
        if let Some(previous) = unique.last()
            && previous.digest == object.digest
        {
            if previous != &object {
                return Err(corrupt("one CAS digest has conflicting metadata"));
            }
            continue;
        }
        unique.push(object);
    }
    Ok(unique)
}

fn object_order(left: &CasObject, right: &CasObject) -> std::cmp::Ordering {
    (&left.digest, &left.object_schema, left.size).cmp(&(
        &right.digest,
        &right.object_schema,
        right.size,
    ))
}

fn verify_objects(store: &CasStore, objects: &[CasObject]) -> Result<(), ClewError> {
    for object in objects {
        let limit = usize::try_from(object.size).map_err(|_| {
            ClewError::new(ErrorCode::ResourceLimit, "derived input exceeds host size")
        })?;
        store.read(object, limit)?;
    }
    Ok(())
}

fn invalid(message: &str) -> ClewError {
    ClewError::new(ErrorCode::InvalidInput, message)
}

fn corrupt(message: &str) -> ClewError {
    ClewError::new(ErrorCode::StateCorrupt, message)
}

fn internal(error: impl std::fmt::Display) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter_v2::{
        COMPILATION_SCHEMA, DescriptorCompleteness, DescriptorOrigin, LanguageUri,
        PROVIDER_PROTOCOL, ProviderHandshake, SourceRootDescriptor,
    };
    use crate::state::StateAuthority;

    fn fixture() -> (tempfile::TempDir, CasStore) {
        let root = tempfile::tempdir().unwrap();
        let authority = StateAuthority::open(root.path().join("v2")).unwrap();
        let store = CasStore::open(&authority).unwrap();
        (root, store)
    }

    fn compilation(store: &CasStore, id: &str, dependency: Option<&str>) -> CompilationDescriptor {
        CompilationDescriptor {
            schema: COMPILATION_SCHEMA.into(),
            compilation_id: id.into(),
            language_uri: LanguageUri::parse("language:zeta").unwrap(),
            source_roots: vec![SourceRootDescriptor {
                logical_name: "main".into(),
                tree: store.put("test/tree/1", id.as_bytes()).unwrap(),
            }],
            generated_source_roots: vec![],
            classpath: vec![],
            toolchain: store.put("test/toolchain/1", b"zeta-1").unwrap(),
            plugins: vec![],
            canonical_options: store.put("test/options/1", b"{}").unwrap(),
            dependency_compilation_ids: dependency.into_iter().map(str::to_owned).collect(),
            operations: vec![],
            origin: DescriptorOrigin::ProjectNative,
            completeness: DescriptorCompleteness::Complete,
        }
    }

    fn provider(
        store: &CasStore,
        id: &str,
        compilations: Vec<CompilationDescriptor>,
    ) -> ProviderModel {
        ProviderModel {
            handshake: ProviderHandshake {
                protocol: PROVIDER_PROTOCOL.into(),
                provider_id: id.into(),
                provider_digest: format!("sha256:{}", "a".repeat(64)),
                build_system_uris: vec!["build:fake".into()],
            },
            build_model: crate::adapter_v2::BuildModel {
                provider_id: id.into(),
                model: store.put("test/model/1", id.as_bytes()).unwrap(),
                compilations,
            },
        }
    }

    #[test]
    fn arrival_order_cannot_change_manifest_or_object_identity() {
        let (_root, store) = fixture();
        let snapshot = store.put(SNAPSHOT_SCHEMA, b"snapshot").unwrap();
        let left = provider(&store, "left", vec![compilation(&store, "core", None)]);
        let right = provider(
            &store,
            "right",
            vec![compilation(&store, "app", Some("core"))],
        );
        let (first, first_object) = DerivedAnalysisInputManifest::create(
            &store,
            snapshot.clone(),
            vec![right.clone(), left.clone()],
        )
        .unwrap();
        let (second, second_object) =
            DerivedAnalysisInputManifest::create(&store, snapshot, vec![left, right]).unwrap();
        assert_eq!(first, second);
        assert_eq!(first_object, second_object);
        first.verify(&store).unwrap();
    }

    #[test]
    fn missing_or_cyclic_compilation_dependency_fails_closed() {
        let (_root, store) = fixture();
        let snapshot = store.put(SNAPSHOT_SCHEMA, b"snapshot").unwrap();
        let missing = provider(
            &store,
            "build",
            vec![compilation(&store, "app", Some("missing"))],
        );
        assert_eq!(
            DerivedAnalysisInputManifest::create(&store, snapshot.clone(), vec![missing])
                .unwrap_err()
                .code,
            ErrorCode::InvalidInput
        );
        let cycle = provider(
            &store,
            "build",
            vec![
                compilation(&store, "a", Some("b")),
                compilation(&store, "b", Some("a")),
            ],
        );
        assert_eq!(
            DerivedAnalysisInputManifest::create(&store, snapshot, vec![cycle])
                .unwrap_err()
                .code,
            ErrorCode::InvalidInput
        );
    }
}
