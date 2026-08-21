use crate::cas::{CAS_OBJECT_SCHEMA, CasObject, CasStore};
use crate::error::{ClewError, ErrorCode};
use rayon::prelude::*;
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

pub const PROVIDER_PROTOCOL: &str = "codeclew-build-model-provider/2.0";
pub const ADAPTER_PROTOCOL: &str = "codeclew-language-adapter/2.0";
pub const COMPILATION_SCHEMA: &str = "codeclew-compilation-descriptor/2.0";
pub const ANALYSIS_REQUEST_SCHEMA: &str = "codeclew-analyze-generation-request/2.0";

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct LanguageUri(String);

impl LanguageUri {
    pub fn parse(value: impl Into<String>) -> Result<Self, ClewError> {
        let value = value.into();
        validate_uri(&value, "language URI")?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for LanguageUri {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct CapabilityUri(String);

impl CapabilityUri {
    pub fn parse(value: impl Into<String>) -> Result<Self, ClewError> {
        let value = value.into();
        validate_uri(&value, "capability URI")?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for CapabilityUri {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolchainConstraint {
    pub authority_digest: String,
    pub minimum_version: Option<String>,
    pub maximum_version_exclusive: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DescriptorOrigin {
    ProjectNative,
    SealedExternal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DescriptorCompleteness {
    Complete,
    Partial,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceRootDescriptor {
    pub logical_name: String,
    pub tree: CasObject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperationDescriptor {
    pub operation_uri: CapabilityUri,
    pub arguments: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompilationDescriptor {
    pub schema: String,
    pub compilation_id: String,
    pub language_uri: LanguageUri,
    pub source_roots: Vec<SourceRootDescriptor>,
    pub generated_source_roots: Vec<SourceRootDescriptor>,
    pub classpath: Vec<CasObject>,
    pub toolchain: CasObject,
    pub plugins: Vec<CasObject>,
    pub canonical_options: CasObject,
    pub dependency_compilation_ids: Vec<String>,
    pub operations: Vec<OperationDescriptor>,
    pub origin: DescriptorOrigin,
    pub completeness: DescriptorCompleteness,
}

impl CompilationDescriptor {
    pub fn validate(&self) -> Result<(), ClewError> {
        if self.schema != COMPILATION_SCHEMA || !safe_id(&self.compilation_id) {
            return Err(invalid("compilation descriptor identity is invalid"));
        }
        validate_cas(&self.toolchain)?;
        validate_cas(&self.canonical_options)?;
        if self.source_roots.is_empty() || self.source_roots.len() > 4096 {
            return Err(invalid("compilation source root set is invalid"));
        }
        let mut root_names = BTreeSet::new();
        for root in self.source_roots.iter().chain(&self.generated_source_roots) {
            if !safe_id(&root.logical_name) || !root_names.insert(&root.logical_name) {
                return Err(invalid("compilation source roots are not uniquely named"));
            }
            validate_cas(&root.tree)?;
        }
        validate_unique_cas(&self.classpath, "classpath")?;
        validate_unique_cas(&self.plugins, "plugins")?;
        let mut dependencies = BTreeSet::new();
        for dependency in &self.dependency_compilation_ids {
            if !safe_id(dependency)
                || dependency == &self.compilation_id
                || !dependencies.insert(dependency)
            {
                return Err(invalid("compilation dependency set is invalid"));
            }
        }
        let mut operations = BTreeSet::new();
        for operation in &self.operations {
            if !operations.insert(operation.operation_uri.as_str())
                || operation.arguments.len() > 1024
                || operation.arguments.iter().any(|argument| {
                    argument.len() > 4096
                        || argument.contains('\0')
                        || argument.starts_with('/')
                        || argument.split('/').any(|component| component == "..")
                })
            {
                return Err(invalid("compilation operation is unsafe or duplicated"));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderHandshake {
    pub protocol: String,
    pub provider_id: String,
    pub provider_digest: String,
    pub build_system_uris: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AdapterHandshake {
    pub protocol: String,
    pub adapter_id: String,
    pub adapter_digest: String,
    pub languages: Vec<LanguageUri>,
    pub capabilities: Vec<CapabilityUri>,
    pub toolchains: Vec<ToolchainConstraint>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProbeRequest {
    pub repository_snapshot: CasObject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProbeDecision {
    Supported,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelRequest {
    pub repository_snapshot: CasObject,
    pub requested_compilations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BuildModel {
    pub provider_id: String,
    pub model: CasObject,
    pub compilations: Vec<CompilationDescriptor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderModel {
    pub handshake: ProviderHandshake,
    pub build_model: BuildModel,
}

pub trait BuildModelProvider: Send + Sync {
    fn handshake(&self) -> Result<ProviderHandshake, ClewError>;
    fn probe(&self, request: &ProbeRequest) -> Result<ProbeDecision, ClewError>;
    fn extract_model(&self, request: &ModelRequest) -> Result<BuildModel, ClewError>;
    fn shutdown(&self) -> Result<(), ClewError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AnalyzeGenerationRequest {
    pub schema: String,
    pub attempt_id: String,
    pub generation_key: String,
    pub capability: CapabilityUri,
    pub compilation: CompilationDescriptor,
    pub derived_input_manifest: CasObject,
    pub parent_generation: Option<CasObject>,
}

impl AnalyzeGenerationRequest {
    pub fn validate(&self) -> Result<(), ClewError> {
        if self.schema != ANALYSIS_REQUEST_SCHEMA || !safe_id(&self.attempt_id) {
            return Err(invalid("analysis request identity is invalid"));
        }
        digest_component(&self.generation_key)?;
        self.compilation.validate()?;
        validate_cas(&self.derived_input_manifest)?;
        if let Some(parent) = &self.parent_generation {
            validate_cas(parent)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FactRecord {
    pub fact_key: String,
    pub domain_uri: CapabilityUri,
    pub payload: CasObject,
}

impl FactRecord {
    pub fn validate(&self) -> Result<(), ClewError> {
        if !safe_fact_key(&self.fact_key) {
            return Err(invalid("fact key is invalid"));
        }
        validate_cas(&self.payload)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FactShard {
    pub sequence: u32,
    pub facts: Vec<FactRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AnalysisAttemptComplete {
    pub scope_digest: String,
    pub completeness_receipt: CasObject,
    pub fact_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", content = "value", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AnalysisEvent {
    FactShard(FactShard),
    AttemptComplete(AnalysisAttemptComplete),
}

pub trait AnalysisSink {
    fn accept(&mut self, event: AnalysisEvent) -> Result<(), ClewError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QueryGenerationRequest {
    pub generation: CasObject,
    pub capability: CapabilityUri,
    pub keys: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QueryGenerationResult {
    pub generation: CasObject,
    pub facts: Vec<FactRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ValidateCandidateRequest {
    pub candidate_snapshot: CasObject,
    pub generation: CasObject,
    pub operations: Vec<CapabilityUri>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ValidateCandidateResult {
    pub validated: bool,
    pub evidence: Vec<CasObject>,
}

pub trait LanguageAdapter: Send + Sync {
    fn handshake(&self) -> Result<AdapterHandshake, ClewError>;
    fn analyze_generation(
        &self,
        request: &AnalyzeGenerationRequest,
        sink: &mut dyn AnalysisSink,
        cancelled: &AtomicBool,
    ) -> Result<(), ClewError>;
    fn query_generation(
        &self,
        request: &QueryGenerationRequest,
    ) -> Result<QueryGenerationResult, ClewError>;
    fn validate_candidate(
        &self,
        request: &ValidateCandidateRequest,
    ) -> Result<ValidateCandidateResult, ClewError>;
    fn cancel(&self, attempt_id: &str) -> Result<(), ClewError>;
    fn shutdown(&self) -> Result<(), ClewError>;
}

#[derive(Default)]
pub struct AdapterRegistry {
    providers: BTreeMap<String, RegisteredProvider>,
    adapters: BTreeMap<String, RegisteredAdapter>,
}

struct RegisteredProvider {
    handshake: ProviderHandshake,
    provider: Arc<dyn BuildModelProvider>,
}

struct RegisteredAdapter {
    handshake: AdapterHandshake,
    adapter: Arc<dyn LanguageAdapter>,
}

impl AdapterRegistry {
    pub fn register_provider(
        &mut self,
        provider: Arc<dyn BuildModelProvider>,
    ) -> Result<(), ClewError> {
        let mut handshake = provider.handshake()?;
        validate_provider_handshake(&mut handshake)?;
        if self.providers.contains_key(&handshake.provider_id) {
            return Err(invalid("build-model provider id is already registered"));
        }
        self.providers.insert(
            handshake.provider_id.clone(),
            RegisteredProvider {
                handshake,
                provider,
            },
        );
        Ok(())
    }

    pub fn register_adapter(&mut self, adapter: Arc<dyn LanguageAdapter>) -> Result<(), ClewError> {
        let mut handshake = adapter.handshake()?;
        validate_adapter_handshake(&mut handshake)?;
        if self.adapters.contains_key(&handshake.adapter_id) {
            return Err(invalid("language adapter id is already registered"));
        }
        self.adapters.insert(
            handshake.adapter_id.clone(),
            RegisteredAdapter { handshake, adapter },
        );
        Ok(())
    }

    pub fn provider(&self, provider_id: &str) -> Result<Arc<dyn BuildModelProvider>, ClewError> {
        self.providers
            .get(provider_id)
            .map(|registered| Arc::clone(&registered.provider))
            .ok_or_else(|| {
                ClewError::new(
                    ErrorCode::UnsupportedProjectConfiguration,
                    "build-model provider is not registered",
                )
            })
    }

    pub fn extract_model(
        &self,
        provider_id: &str,
        request: &ModelRequest,
    ) -> Result<BuildModel, ClewError> {
        validate_cas(&request.repository_snapshot)?;
        let provider = self.provider(provider_id)?;
        let model = provider.extract_model(request)?;
        validate_build_model(provider_id, &model)?;
        Ok(model)
    }

    pub fn extract_supported_models_sealed(
        &self,
        store: &CasStore,
        repository_snapshot: &CasObject,
        requested_compilations: Vec<String>,
    ) -> Result<Vec<ProviderModel>, ClewError> {
        validate_cas(repository_snapshot)?;
        let registrations = self
            .providers
            .values()
            .map(|registered| {
                (
                    registered.handshake.clone(),
                    Arc::clone(&registered.provider),
                )
            })
            .collect::<Vec<_>>();
        let mut models = registrations
            .into_par_iter()
            .map(
                |(handshake, provider)| -> Result<Option<ProviderModel>, ClewError> {
                    let before = read_snapshot_authority(store, repository_snapshot)?;
                    let probe = provider.probe(&ProbeRequest {
                        repository_snapshot: repository_snapshot.clone(),
                    })?;
                    let model = match probe {
                        ProbeDecision::Unsupported => None,
                        ProbeDecision::Supported => {
                            let model = provider.extract_model(&ModelRequest {
                                repository_snapshot: repository_snapshot.clone(),
                                requested_compilations: requested_compilations.clone(),
                            })?;
                            validate_build_model(&handshake.provider_id, &model)?;
                            Some(ProviderModel {
                                handshake,
                                build_model: model,
                            })
                        }
                    };
                    let after =
                        read_snapshot_authority(store, repository_snapshot).map_err(|_| {
                            ClewError::new(
                                ErrorCode::InputMutated,
                                "repository snapshot authority changed during provider execution",
                            )
                        })?;
                    if before != after {
                        return Err(ClewError::new(
                            ErrorCode::InputMutated,
                            "repository snapshot authority changed during provider execution",
                        ));
                    }
                    Ok(model)
                },
            )
            .collect::<Result<Vec<_>, ClewError>>()?
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        models.sort_by(|left, right| left.handshake.provider_id.cmp(&right.handshake.provider_id));
        if models.is_empty() {
            return Err(ClewError::new(
                ErrorCode::UnsupportedProjectConfiguration,
                "no build-model provider supports the sealed repository snapshot",
            ));
        }
        Ok(models)
    }

    pub fn select_adapter(
        &self,
        language: &LanguageUri,
        capability: &CapabilityUri,
        toolchain_digest: &str,
    ) -> Result<Arc<dyn LanguageAdapter>, ClewError> {
        digest_component(toolchain_digest)?;
        self.select_registered(language, capability, toolchain_digest)
            .map(|registered| Arc::clone(&registered.adapter))
    }

    pub fn analyze_generation(
        &self,
        request: &AnalyzeGenerationRequest,
        cancelled: &AtomicBool,
    ) -> Result<AnalysisAttemptComplete, ClewError> {
        request.validate()?;
        let registered = self.select_registered(
            &request.compilation.language_uri,
            &request.capability,
            &request.compilation.toolchain.digest,
        )?;
        let mut sink =
            ConformanceSink::for_capabilities(registered.handshake.capabilities.iter().cloned());
        registered
            .adapter
            .analyze_generation(request, &mut sink, cancelled)?;
        sink.finish()
    }

    fn select_registered(
        &self,
        language: &LanguageUri,
        capability: &CapabilityUri,
        toolchain_digest: &str,
    ) -> Result<&RegisteredAdapter, ClewError> {
        digest_component(toolchain_digest)?;
        let matches = self
            .adapters
            .values()
            .filter(|registered| {
                registered.handshake.languages.contains(language)
                    && registered.handshake.capabilities.contains(capability)
                    && registered
                        .handshake
                        .toolchains
                        .iter()
                        .any(|constraint| constraint.authority_digest == toolchain_digest)
            })
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [registered] => Ok(*registered),
            [] => Err(ClewError::new(
                ErrorCode::UnsupportedLanguage,
                "no language adapter satisfies the language, capability, and toolchain authority",
            )),
            _ => Err(invalid(
                "multiple language adapters satisfy one exact authority",
            )),
        }
    }

    pub fn provider_handshakes(&self) -> impl Iterator<Item = &ProviderHandshake> {
        self.providers.values().map(|value| &value.handshake)
    }

    pub fn adapter_handshakes(&self) -> impl Iterator<Item = &AdapterHandshake> {
        self.adapters.values().map(|value| &value.handshake)
    }
}

fn read_snapshot_authority(store: &CasStore, object: &CasObject) -> Result<Vec<u8>, ClewError> {
    let limit = usize::try_from(object.size).map_err(|_| {
        ClewError::new(ErrorCode::ResourceLimit, "repository snapshot is too large")
    })?;
    Ok(store.read(object, limit)?.bytes().to_vec())
}

fn validate_build_model(provider_id: &str, model: &BuildModel) -> Result<(), ClewError> {
    if model.provider_id != provider_id {
        return Err(protocol("provider returned a model with another identity"));
    }
    validate_cas(&model.model)?;
    if model.compilations.is_empty() || model.compilations.len() > 65_536 {
        return Err(protocol("provider returned an invalid compilation set"));
    }
    let mut compilation_ids = BTreeSet::new();
    for compilation in &model.compilations {
        compilation.validate()?;
        if !compilation_ids.insert(&compilation.compilation_id) {
            return Err(protocol("provider returned duplicate compilation ids"));
        }
    }
    Ok(())
}

#[derive(Default)]
pub struct ConformanceSink {
    next_sequence: u32,
    fact_count: u64,
    last_fact_key: Option<String>,
    allowed_capabilities: Option<BTreeSet<CapabilityUri>>,
    complete: Option<AnalysisAttemptComplete>,
}

impl ConformanceSink {
    pub fn for_capabilities(capabilities: impl IntoIterator<Item = CapabilityUri>) -> Self {
        Self {
            allowed_capabilities: Some(capabilities.into_iter().collect()),
            ..Self::default()
        }
    }

    pub fn finish(self) -> Result<AnalysisAttemptComplete, ClewError> {
        let complete = self.complete.ok_or_else(|| {
            ClewError::new(
                ErrorCode::WorkerProtocolMismatch,
                "adapter stream ended without AnalysisAttemptComplete",
            )
        })?;
        if complete.fact_count != self.fact_count {
            return Err(ClewError::new(
                ErrorCode::WorkerProtocolMismatch,
                "adapter completion fact count differs from streamed facts",
            ));
        }
        Ok(complete)
    }
}

impl AnalysisSink for ConformanceSink {
    fn accept(&mut self, event: AnalysisEvent) -> Result<(), ClewError> {
        if self.complete.is_some() {
            return Err(ClewError::new(
                ErrorCode::WorkerProtocolMismatch,
                "adapter emitted data after AnalysisAttemptComplete",
            ));
        }
        match event {
            AnalysisEvent::FactShard(shard) => {
                if shard.sequence != self.next_sequence || shard.facts.is_empty() {
                    return Err(protocol("adapter shard sequence or size is invalid"));
                }
                for fact in &shard.facts {
                    if !safe_fact_key(&fact.fact_key)
                        || self
                            .last_fact_key
                            .as_deref()
                            .is_some_and(|value| value >= fact.fact_key.as_str())
                    {
                        return Err(protocol("adapter facts are not strictly sorted"));
                    }
                    if self
                        .allowed_capabilities
                        .as_ref()
                        .is_some_and(|allowed| !allowed.contains(&fact.domain_uri))
                    {
                        return Err(protocol(
                            "adapter emitted a fact outside its declared capabilities",
                        ));
                    }
                    validate_cas(&fact.payload)?;
                    self.last_fact_key = Some(fact.fact_key.clone());
                }
                self.next_sequence += 1;
                self.fact_count += shard.facts.len() as u64;
            }
            AnalysisEvent::AttemptComplete(complete) => {
                digest_component(&complete.scope_digest)?;
                validate_cas(&complete.completeness_receipt)?;
                self.complete = Some(complete);
            }
        }
        Ok(())
    }
}

fn validate_provider_handshake(handshake: &mut ProviderHandshake) -> Result<(), ClewError> {
    if handshake.protocol != PROVIDER_PROTOCOL
        || !safe_id(&handshake.provider_id)
        || digest_component(&handshake.provider_digest).is_err()
        || handshake.build_system_uris.is_empty()
    {
        return Err(protocol("build-model provider handshake is invalid"));
    }
    handshake.build_system_uris.sort();
    handshake.build_system_uris.dedup();
    for uri in &handshake.build_system_uris {
        validate_uri(uri, "build system URI")?;
    }
    Ok(())
}

fn validate_adapter_handshake(handshake: &mut AdapterHandshake) -> Result<(), ClewError> {
    if handshake.protocol != ADAPTER_PROTOCOL
        || !safe_id(&handshake.adapter_id)
        || digest_component(&handshake.adapter_digest).is_err()
        || handshake.languages.is_empty()
        || handshake.capabilities.is_empty()
        || handshake.toolchains.is_empty()
    {
        return Err(protocol("language adapter handshake is invalid"));
    }
    handshake.languages.sort();
    handshake.languages.dedup();
    handshake.capabilities.sort();
    handshake.capabilities.dedup();
    let mut toolchains = BTreeSet::new();
    for constraint in &handshake.toolchains {
        digest_component(&constraint.authority_digest)?;
        for version in [
            constraint.minimum_version.as_deref(),
            constraint.maximum_version_exclusive.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            if version.is_empty()
                || version.len() > 128
                || version
                    .bytes()
                    .any(|byte| byte.is_ascii_control() || byte == b' ')
            {
                return Err(protocol("language adapter toolchain range is invalid"));
            }
        }
        if !toolchains.insert(&constraint.authority_digest) {
            return Err(protocol(
                "language adapter toolchain authority is duplicated",
            ));
        }
    }
    Ok(())
}

fn validate_unique_cas(objects: &[CasObject], label: &str) -> Result<(), ClewError> {
    if objects.len() > 65_536 {
        return Err(invalid(&format!("compilation {label} exceeds its bound")));
    }
    let mut digests = BTreeSet::new();
    for object in objects {
        validate_cas(object)?;
        if !digests.insert(&object.digest) {
            return Err(invalid(&format!(
                "compilation {label} contains duplicate CAS objects"
            )));
        }
    }
    Ok(())
}

fn validate_cas(object: &CasObject) -> Result<(), ClewError> {
    if object.schema != CAS_OBJECT_SCHEMA || object.object_schema.is_empty() {
        return Err(invalid("CAS reference schema is invalid"));
    }
    digest_component(&object.digest).map(|_| ())
}

fn validate_uri(value: &str, label: &str) -> Result<(), ClewError> {
    if value.len() > 256
        || !value.contains(':')
        || value
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte == b' ')
    {
        return Err(invalid(&format!("{label} is invalid")));
    }
    Ok(())
}

fn digest_component(value: &str) -> Result<&str, ClewError> {
    let component = value
        .strip_prefix("sha256:")
        .ok_or_else(|| invalid("authority digest has no sha256 prefix"))?;
    if component.len() != 64
        || !component
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(invalid("authority digest is not canonical sha256"));
    }
    Ok(component)
}

fn safe_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}

fn safe_fact_key(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 4096
        && !value
            .bytes()
            .any(|byte| byte == 0 || byte.is_ascii_control())
}

fn invalid(message: &str) -> ClewError {
    ClewError::new(ErrorCode::InvalidInput, message)
}

fn protocol(message: &str) -> ClewError {
    ClewError::new(ErrorCode::WorkerProtocolMismatch, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::StateAuthority;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn digest(byte: char) -> String {
        format!("sha256:{}", byte.to_string().repeat(64))
    }

    fn object(schema: &str, byte: char) -> CasObject {
        CasObject {
            schema: CAS_OBJECT_SCHEMA.into(),
            object_schema: schema.into(),
            digest: digest(byte),
            size: 1,
        }
    }

    fn zeta_compilation() -> CompilationDescriptor {
        CompilationDescriptor {
            schema: COMPILATION_SCHEMA.into(),
            compilation_id: "zeta-main".into(),
            language_uri: LanguageUri::parse("language:zeta").unwrap(),
            source_roots: vec![SourceRootDescriptor {
                logical_name: "main".into(),
                tree: object("tree/1", '1'),
            }],
            generated_source_roots: vec![],
            classpath: vec![],
            toolchain: object("toolchain/1", '2'),
            plugins: vec![],
            canonical_options: object("options/1", '3'),
            dependency_compilation_ids: vec![],
            operations: vec![OperationDescriptor {
                operation_uri: CapabilityUri::parse("operation:test").unwrap(),
                arguments: vec!["zetaTest".into()],
            }],
            origin: DescriptorOrigin::ProjectNative,
            completeness: DescriptorCompleteness::Complete,
        }
    }

    struct ZetaProvider;

    impl BuildModelProvider for ZetaProvider {
        fn handshake(&self) -> Result<ProviderHandshake, ClewError> {
            Ok(ProviderHandshake {
                protocol: PROVIDER_PROTOCOL.into(),
                provider_id: "fake-build".into(),
                provider_digest: digest('a'),
                build_system_uris: vec!["build:fake".into()],
            })
        }

        fn probe(&self, _request: &ProbeRequest) -> Result<ProbeDecision, ClewError> {
            Ok(ProbeDecision::Supported)
        }

        fn extract_model(&self, _request: &ModelRequest) -> Result<BuildModel, ClewError> {
            Ok(BuildModel {
                provider_id: "fake-build".into(),
                model: object("model/1", 'b'),
                compilations: vec![zeta_compilation()],
            })
        }

        fn shutdown(&self) -> Result<(), ClewError> {
            Ok(())
        }
    }

    struct ZetaAdapter;

    impl LanguageAdapter for ZetaAdapter {
        fn handshake(&self) -> Result<AdapterHandshake, ClewError> {
            Ok(AdapterHandshake {
                protocol: ADAPTER_PROTOCOL.into(),
                adapter_id: "fake-zeta".into(),
                adapter_digest: digest('c'),
                languages: vec![LanguageUri::parse("language:zeta").unwrap()],
                capabilities: vec![CapabilityUri::parse("analysis:facts").unwrap()],
                toolchains: vec![ToolchainConstraint {
                    authority_digest: digest('2'),
                    minimum_version: None,
                    maximum_version_exclusive: None,
                }],
            })
        }

        fn analyze_generation(
            &self,
            _request: &AnalyzeGenerationRequest,
            sink: &mut dyn AnalysisSink,
            cancelled: &AtomicBool,
        ) -> Result<(), ClewError> {
            if cancelled.load(Ordering::Acquire) {
                return Err(ClewError::new(
                    ErrorCode::TransactionRecoveryRequired,
                    "cancelled",
                ));
            }
            sink.accept(AnalysisEvent::FactShard(FactShard {
                sequence: 0,
                facts: vec![FactRecord {
                    fact_key: "zeta:main".into(),
                    domain_uri: CapabilityUri::parse("analysis:facts").unwrap(),
                    payload: object("fact/1", 'd'),
                }],
            }))?;
            sink.accept(AnalysisEvent::AttemptComplete(AnalysisAttemptComplete {
                scope_digest: digest('e'),
                completeness_receipt: object("receipt/1", 'f'),
                fact_count: 1,
            }))
        }

        fn query_generation(
            &self,
            request: &QueryGenerationRequest,
        ) -> Result<QueryGenerationResult, ClewError> {
            Ok(QueryGenerationResult {
                generation: request.generation.clone(),
                facts: vec![],
            })
        }

        fn validate_candidate(
            &self,
            _request: &ValidateCandidateRequest,
        ) -> Result<ValidateCandidateResult, ClewError> {
            Ok(ValidateCandidateResult {
                validated: true,
                evidence: vec![],
            })
        }

        fn cancel(&self, _attempt_id: &str) -> Result<(), ClewError> {
            Ok(())
        }

        fn shutdown(&self) -> Result<(), ClewError> {
            Ok(())
        }
    }

    struct CountingProvider {
        id: &'static str,
        probes: Arc<AtomicUsize>,
        extractions: Arc<AtomicUsize>,
    }

    impl BuildModelProvider for CountingProvider {
        fn handshake(&self) -> Result<ProviderHandshake, ClewError> {
            Ok(ProviderHandshake {
                protocol: PROVIDER_PROTOCOL.into(),
                provider_id: self.id.into(),
                provider_digest: digest(if self.id == "provider-a" { 'a' } else { 'b' }),
                build_system_uris: vec![format!("build:{}", self.id)],
            })
        }

        fn probe(&self, _request: &ProbeRequest) -> Result<ProbeDecision, ClewError> {
            self.probes.fetch_add(1, Ordering::AcqRel);
            Ok(ProbeDecision::Supported)
        }

        fn extract_model(&self, _request: &ModelRequest) -> Result<BuildModel, ClewError> {
            self.extractions.fetch_add(1, Ordering::AcqRel);
            let mut compilation = zeta_compilation();
            compilation.compilation_id = format!("{}-main", self.id);
            Ok(BuildModel {
                provider_id: self.id.into(),
                model: object("model/1", if self.id == "provider-a" { '4' } else { '5' }),
                compilations: vec![compilation],
            })
        }

        fn shutdown(&self) -> Result<(), ClewError> {
            Ok(())
        }
    }

    #[test]
    fn fake_language_registers_without_core_changes_and_streams_complete_generation() {
        let mut registry = AdapterRegistry::default();
        registry.register_provider(Arc::new(ZetaProvider)).unwrap();
        registry.register_adapter(Arc::new(ZetaAdapter)).unwrap();
        let model = registry
            .extract_model(
                "fake-build",
                &ModelRequest {
                    repository_snapshot: object("snapshot/1", '0'),
                    requested_compilations: vec![],
                },
            )
            .unwrap();
        assert_eq!(model.compilations.len(), 1);
        model.compilations[0].validate().unwrap();
        let capability = CapabilityUri::parse("analysis:facts").unwrap();
        let completion = registry
            .analyze_generation(
                &AnalyzeGenerationRequest {
                    schema: ANALYSIS_REQUEST_SCHEMA.into(),
                    attempt_id: "attempt:fake".into(),
                    generation_key: digest('9'),
                    capability,
                    compilation: model.compilations[0].clone(),
                    derived_input_manifest: object("derived/1", '8'),
                    parent_generation: None,
                },
                &AtomicBool::new(false),
            )
            .unwrap();
        assert_eq!(completion.fact_count, 1);
    }

    #[test]
    fn supported_providers_execute_once_and_return_in_canonical_order() {
        let state = tempfile::tempdir().unwrap();
        let authority = StateAuthority::open(state.path().join("v2")).unwrap();
        let store = CasStore::open(&authority).unwrap();
        let snapshot = store.put("snapshot/2", b"sealed").unwrap();
        let probes = Arc::new(AtomicUsize::new(0));
        let extractions = Arc::new(AtomicUsize::new(0));
        let mut registry = AdapterRegistry::default();
        for id in ["provider-b", "provider-a"] {
            registry
                .register_provider(Arc::new(CountingProvider {
                    id,
                    probes: Arc::clone(&probes),
                    extractions: Arc::clone(&extractions),
                }))
                .unwrap();
        }
        let models = registry
            .extract_supported_models_sealed(&store, &snapshot, vec![])
            .unwrap();
        assert_eq!(probes.load(Ordering::Acquire), 2);
        assert_eq!(extractions.load(Ordering::Acquire), 2);
        assert_eq!(models[0].handshake.provider_id, "provider-a");
        assert_eq!(models[1].handshake.provider_id, "provider-b");
    }

    #[test]
    fn adapter_selection_is_exact_and_ambiguity_fails_closed() {
        let mut registry = AdapterRegistry::default();
        registry.register_adapter(Arc::new(ZetaAdapter)).unwrap();
        let missing = registry.select_adapter(
            &LanguageUri::parse("language:other").unwrap(),
            &CapabilityUri::parse("analysis:facts").unwrap(),
            &digest('2'),
        );
        let error = match missing {
            Ok(_) => panic!("unsupported language unexpectedly selected an adapter"),
            Err(error) => error,
        };
        assert_eq!(error.code, ErrorCode::UnsupportedLanguage);

        struct SecondZeta;
        impl LanguageAdapter for SecondZeta {
            fn handshake(&self) -> Result<AdapterHandshake, ClewError> {
                let mut value = ZetaAdapter.handshake()?;
                value.adapter_id = "fake-zeta-second".into();
                Ok(value)
            }
            fn analyze_generation(
                &self,
                request: &AnalyzeGenerationRequest,
                sink: &mut dyn AnalysisSink,
                cancelled: &AtomicBool,
            ) -> Result<(), ClewError> {
                ZetaAdapter.analyze_generation(request, sink, cancelled)
            }
            fn query_generation(
                &self,
                request: &QueryGenerationRequest,
            ) -> Result<QueryGenerationResult, ClewError> {
                ZetaAdapter.query_generation(request)
            }
            fn validate_candidate(
                &self,
                request: &ValidateCandidateRequest,
            ) -> Result<ValidateCandidateResult, ClewError> {
                ZetaAdapter.validate_candidate(request)
            }
            fn cancel(&self, attempt_id: &str) -> Result<(), ClewError> {
                ZetaAdapter.cancel(attempt_id)
            }
            fn shutdown(&self) -> Result<(), ClewError> {
                Ok(())
            }
        }
        registry.register_adapter(Arc::new(SecondZeta)).unwrap();
        assert!(
            registry
                .select_adapter(
                    &LanguageUri::parse("language:zeta").unwrap(),
                    &CapabilityUri::parse("analysis:facts").unwrap(),
                    &digest('2'),
                )
                .is_err()
        );
    }

    #[test]
    fn stream_refuses_unsorted_facts_and_data_after_completion() {
        let mut sink = ConformanceSink::default();
        let unsorted = sink.accept(AnalysisEvent::FactShard(FactShard {
            sequence: 0,
            facts: vec![
                FactRecord {
                    fact_key: "z".into(),
                    domain_uri: CapabilityUri::parse("analysis:facts").unwrap(),
                    payload: object("fact/1", '1'),
                },
                FactRecord {
                    fact_key: "a".into(),
                    domain_uri: CapabilityUri::parse("analysis:facts").unwrap(),
                    payload: object("fact/1", '2'),
                },
            ],
        }));
        assert!(unsorted.is_err());

        let mut sink = ConformanceSink::default();
        sink.accept(AnalysisEvent::AttemptComplete(AnalysisAttemptComplete {
            scope_digest: digest('3'),
            completeness_receipt: object("receipt/1", '4'),
            fact_count: 0,
        }))
        .unwrap();
        assert!(
            sink.accept(AnalysisEvent::FactShard(FactShard {
                sequence: 0,
                facts: vec![],
            }))
            .is_err()
        );
    }

    #[test]
    fn descriptor_refuses_absolute_operations_and_duplicate_roots() {
        let mut descriptor = zeta_compilation();
        descriptor.operations[0].arguments = vec!["/private/project".into()];
        assert!(descriptor.validate().is_err());
        let mut descriptor = zeta_compilation();
        descriptor
            .generated_source_roots
            .push(descriptor.source_roots[0].clone());
        assert!(descriptor.validate().is_err());
    }
}
