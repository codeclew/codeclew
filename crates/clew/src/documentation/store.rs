//! Separate machine-owned records preserve authored YAML comments and manual prose.
use super::{bytes, digest, invalid, io_error, model::*};
use crate::error::{ClewError, ErrorCode};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};

pub const MAX_RECORD: u64 = 128 * 1024 * 1024;
pub const MAX_RECORDS: usize = 1024;

#[derive(Debug)]
pub struct Repository {
    pub root: PathBuf,
    pub manifest: Manifest,
    pub(crate) object_database:
        std::sync::Mutex<Option<(String, std::sync::Arc<super::sqlite_objects::SqliteObjects>)>>,
}

/// Parsed documentation inputs. The serialized fields preserve the existing
/// input digest contract. Capturing this value is a bounded sequential read,
/// not an atomic filesystem snapshot or authority for native analyzer reuse.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RepositoryInputs {
    pub manifest: Manifest,
    pub services: BTreeMap<String, Service>,
    pub interactions: BTreeMap<String, Interaction>,
    pub scenarios: BTreeMap<String, Scenario>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub process_states: BTreeMap<String, super::process_states::ProcessStates>,
    pub entities: BTreeMap<String, super::entities::Entity>,
    pub notes: BTreeMap<String, Value>,
    pub evidence_expectations: BTreeMap<String, super::evidence_package::Expectation>,
    pub update_policies: BTreeMap<String, super::updates::Policy>,
    pub update_state: super::updates::State,
}

pub struct WriteLock(PathBuf);
impl Drop for WriteLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 100
        && id.bytes().next().is_some_and(|c| c.is_ascii_alphabetic())
        && id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}

pub fn relative(path: &str) -> Result<(), ClewError> {
    if path.is_empty()
        || path.contains('\\')
        || path.contains(':')
        || Path::new(path)
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(invalid(
            "documentation path must stay inside its declared root",
        ));
    }
    Ok(())
}

pub fn read<T: DeserializeOwned>(path: &Path, limit: u64) -> Result<T, ClewError> {
    let metadata = fs::symlink_metadata(path).map_err(io_error)?;
    if !metadata.is_file() || metadata.len() > limit {
        return Err(invalid("documentation input is not a bounded regular file"));
    }
    let data = fs::read(path).map_err(io_error)?;
    if data.len() as u64 > limit {
        return Err(invalid("documentation input grew beyond its bound"));
    }
    serde_yaml_ng::from_slice(&data)
        .map_err(|_| invalid("documentation input violates its closed JSON/YAML schema"))
}

pub fn safe_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let host = rest.split('/').next().unwrap_or("");
    !host.is_empty()
        && !host.contains('@')
        && !url.contains(['?', '#', '\\'])
        && !url.chars().any(|c| c.is_whitespace() || c.is_control())
}

impl Repository {
    pub fn init(root: &Path, title: &str) -> Result<Value, ClewError> {
        if title.trim().is_empty() || title.len() > 512 {
            return Err(invalid("documentation title is required"));
        }
        fs::create_dir_all(root).map_err(io_error)?;
        let root = root.canonicalize().map_err(io_error)?;
        let manifest = Manifest {
            schema: format!("codeclew-documentation/{VERSION}"),
            title: title.into(),
        };
        let repo = Self {
            root,
            manifest,
            object_database: Default::default(),
        };
        let path = repo.path("codeclew-docs.yaml")?;
        let new_root = !path.exists();
        if !new_root {
            let current: Manifest = read(&path, MAX_RECORD)?;
            validate_manifest_schema(&current)?;
            if current != repo.manifest {
                return Err(invalid(
                    "documentation root already has a different manifest",
                ));
            }
        }
        // Only an entirely absent private root can denote a fresh Git clone.
        // A partial cache is damaged local state, never an implicit reset.
        let private_state_absent = !repo.path(".codeclew")?.try_exists().map_err(io_error)?;
        let _lock = repo.lock()?;
        if new_root || private_state_absent {
            // Publish the SQLite layout before the new root manifest. If this
            // process is interrupted, a retry still sees an uninitialized root
            // and can complete activation.
            super::object_layout::activate(&repo)?;
        } else {
            super::object_layout::ensure_current(&repo)?;
        }
        if path.exists() {
            let current: Manifest = read(&path, MAX_RECORD)?;
            if current != repo.manifest {
                return Err(invalid(
                    "documentation root already has a different manifest",
                ));
            }
        } else {
            let encoded = serde_yaml_ng::to_string(&repo.manifest).map_err(io_error)?;
            repo.atomic("codeclew-docs.yaml", encoded.as_bytes())?;
        }
        for directory in [
            "catalog/services",
            "catalog/interactions",
            "scenarios",
            "narratives",
            "docs",
            "evidence",
            ".codeclew/bindings",
            ".codeclew/cache",
        ] {
            fs::create_dir_all(repo.path(directory)?).map_err(io_error)?;
        }
        let ignore = repo.path(".gitignore")?;
        let old = if ignore.exists() {
            fs::read_to_string(&ignore).map_err(io_error)?
        } else {
            String::new()
        };
        if !old.lines().any(|line| line.trim() == "/.codeclew/") {
            repo.atomic(
                ".gitignore",
                format!(
                    "{}{}/.codeclew/\n",
                    old,
                    if old.is_empty() || old.ends_with('\n') {
                        ""
                    } else {
                        "\n"
                    }
                )
                .as_bytes(),
            )?;
        }
        let instructions = "# Documentation ownership\n\nRead codeclew-docs.yaml and catalog records first. Use clew docs commands for\nvalidated operations. Bind relocated checkouts with clew docs bind.\nManual notes belong outside docs/generated and narratives; never overwrite them.\nDefault service scope covers every discovered entrypoint, including explicit gaps.\nUse current narrative 1.3; earlier narrative formats are unsupported.\nRetained consumers read saved snapshots without automatic source checks.\nPrefer docs work prepare/read and docs proposal submit for local authoring.\nPublish a machine-ready proposal with docs proposal publish --unassessed;\nthis retains captured influence and explicitly leaves meaning review UNASSESSED.\nSeparate configured review is required for VERIFIED meaning.\nUse domain explanation paragraphs linked to every diagram step.\nExplain business inputs, checks, state changes, failures and outcomes from source.\nPreserve callback scheduling and asynchronous message boundaries.\nDeclared interactions are not compiler or runtime proof. Run clew docs check\nbefore refreshing; preserve stable declaration and scenario IDs.\nKotlin/Java source-syntax authoring does not require an optional compiler provider.\nRetain evidence/packages, execution/accounts and immutable history with the docs.\n";
        let instructions_path = if repo.path("AGENTS.md")?.exists() {
            "AGENTS.codeclew-docs.md"
        } else {
            "AGENTS.md"
        };
        if !repo.path(instructions_path)?.exists() {
            repo.atomic(instructions_path, instructions.as_bytes())?;
        }
        for (name, content) in [
            (
                "examples/service-source.json",
                include_str!("../../assets/documentation/examples/service-source.json"),
            ),
            (
                "examples/service.json",
                include_str!("../../assets/documentation/examples/service.json"),
            ),
            (
                "examples/interaction.json",
                include_str!("../../assets/documentation/examples/interaction.json"),
            ),
            (
                "examples/scenario.yaml",
                include_str!("../../assets/documentation/examples/scenario.yaml"),
            ),
        ] {
            if !repo.path(name)?.exists() {
                repo.atomic(name, content.as_bytes())?;
            }
        }
        super::reader::init(&repo)?;
        Ok(
            json!({"schema":"codeclew-docs-init/1.0", "status":"READY", "inputDigest":repo.input_digest()?}),
        )
    }

    pub fn open(root: &Path) -> Result<Self, ClewError> {
        let root = root.canonicalize().map_err(io_error)?;
        let provisional = Self {
            root,
            object_database: Default::default(),
            manifest: Manifest {
                schema: String::new(),
                title: String::new(),
            },
        };
        let manifest: Manifest = read(&provisional.path("codeclew-docs.yaml")?, MAX_RECORD)?;
        validate_manifest_schema(&manifest)?;
        if !provisional
            .path(".codeclew")?
            .try_exists()
            .map_err(io_error)?
        {
            return Err(invalid(
                "documentation checkout has no local state; run docs init with the existing title, then bind sources and run docs check explicitly",
            ));
        }
        super::object_layout::ensure_current(&provisional)?;
        Ok(Self {
            manifest,
            ..provisional
        })
    }

    pub fn path(&self, relative_path: &str) -> Result<PathBuf, ClewError> {
        relative(relative_path)?;
        let mut path = self.root.clone();
        for segment in Path::new(relative_path).components() {
            path.push(segment.as_os_str());
            match fs::symlink_metadata(&path) {
                Ok(m) if m.file_type().is_symlink() => {
                    return Err(invalid(
                        "symlinks are not supported inside the documentation output root",
                    ));
                }
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(io_error(e)),
            }
        }
        Ok(path)
    }

    pub fn lock(&self) -> Result<WriteLock, ClewError> {
        fs::create_dir_all(self.path(".codeclew")?).map_err(io_error)?;
        let path = self.path(".codeclew/write.lock")?;
        let mut file = OpenOptions::new().write(true).create_new(true).open(&path)
            .map_err(|_| ClewError::new(ErrorCode::WwConflict, "documentation writer lock exists; wait for its owner, or inspect an interrupted writer before removing the lock"))?;
        writeln!(file, "{}", std::process::id()).map_err(io_error)?;
        Ok(WriteLock(path))
    }

    /// Serialize dependent read-modify-write operations with the repository lock.
    /// Independent idempotent signals need no global lock. Tempfiles stay on the same filesystem.
    pub fn atomic(&self, relative_path: &str, data: &[u8]) -> Result<(), ClewError> {
        let path = self.path(relative_path)?;
        let parent = path
            .parent()
            .ok_or_else(|| invalid("missing document parent"))?;
        fs::create_dir_all(parent).map_err(io_error)?;
        if path.exists() && fs::read(&path).map_err(io_error)? == data {
            return Ok(());
        }
        let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(io_error)?;
        temp.write_all(data).map_err(io_error)?;
        temp.as_file().sync_all().map_err(io_error)?;
        self.path(relative_path)?;
        temp.persist(&path).map_err(io_error)?;
        fs::File::open(parent)
            .and_then(|f| f.sync_all())
            .map_err(io_error)
    }

    pub(super) fn records<T: DeserializeOwned>(
        &self,
        directory: &str,
        extension: &str,
    ) -> Result<BTreeMap<String, T>, ClewError> {
        let path = self.path(directory)?;
        let mut result = BTreeMap::new();
        if !path.exists() {
            return Ok(result);
        }
        for entry in fs::read_dir(path).map_err(io_error)? {
            let entry = entry.map_err(io_error)?;
            if entry.path().extension().and_then(|e| e.to_str()) != Some(extension) {
                continue;
            }
            let id = entry
                .path()
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_owned();
            if !valid_id(&id) || result.len() >= MAX_RECORDS {
                return Err(invalid("invalid or excessive documentation records"));
            }
            let record_path = self.path(&format!("{directory}/{id}.{extension}"))?;
            if directory == "scenarios" && extension == "yaml" && id.ends_with("-states") {
                let value: Value = read(&record_path, MAX_RECORD)?;
                if value.get("schema").and_then(Value::as_str)
                    == Some(super::process_states::SCHEMA)
                {
                    // Only the explicit state-schema type is a sidecar. A
                    // scenario whose ordinary ID ends in `-states` remains a
                    // record, as do records in every other directory.
                    continue;
                }
            }
            result.insert(id, read(&record_path, MAX_RECORD)?);
        }
        Ok(result)
    }

    pub fn services(&self) -> Result<BTreeMap<String, Service>, ClewError> {
        let rows: BTreeMap<String, Service> = self.records("catalog/services", "json")?;
        for (id, s) in &rows {
            validate_service(s)?;
            if id != &s.id {
                return Err(invalid("service ID and filename disagree"));
            }
        }
        Ok(rows)
    }
    pub fn interactions(&self) -> Result<BTreeMap<String, Interaction>, ClewError> {
        let rows: BTreeMap<String, Interaction> = self.records("catalog/interactions", "json")?;
        let services = self.services()?;
        for (id, value) in &rows {
            validate_interaction(value, &services)?;
            if id != &value.id {
                return Err(invalid("interaction ID and filename disagree"));
            }
        }
        Ok(rows)
    }
    pub fn scenarios(&self) -> Result<BTreeMap<String, Scenario>, ClewError> {
        let rows: BTreeMap<String, Scenario> = self.records("scenarios", "yaml")?;
        let services = self.services()?;
        let interactions = self.interactions()?;
        for (id, value) in &rows {
            if !matches!(
                value.schema.as_str(),
                "codeclew-documentation-process/1.0" | "codeclew-documentation-view/1.0"
            ) || id != &value.id
                || !valid_id(id)
                || value.max_depth > 16
                || value.max_nodes == 0
                || value.max_nodes > 512
            {
                return Err(invalid("invalid scenario identity or traversal bounds"));
            }
            super::processes::validate(value, &services)?;
            super::dataflow::validate_definition(value, &services)?;
            endpoint(&value.root, &services)?;
            let mut seen = std::collections::BTreeSet::new();
            for interaction in &value.interactions {
                if !interactions.contains_key(interaction) || !seen.insert(interaction) {
                    return Err(invalid("scenario has a dangling or duplicate interaction"));
                }
            }
        }
        Ok(rows)
    }

    pub fn input_digest(&self) -> Result<String, ClewError> {
        digest(&self.inputs()?)
    }

    pub fn inputs(&self) -> Result<RepositoryInputs, ClewError> {
        self.ensure_manifest_current()?;
        // Parse independently authored files without rewriting their bytes/comments.
        let scenarios = self.scenarios()?;
        Ok(RepositoryInputs {
            manifest: self.manifest.clone(),
            services: self.services()?,
            interactions: self.interactions()?,
            process_states: super::process_states::records(self, &scenarios)?,
            scenarios,
            entities: super::entities::records(self)?,
            notes: super::notes::snapshot(self)?,
            evidence_expectations: super::evidence_package::policies(self)?,
            update_policies: super::updates::policies(self)?,
            update_state: super::updates::state(self)?,
        })
    }

    /// Repository instances capture the parsed manifest at open time. Reject
    /// use of an old instance after a meaningful manifest edit so identity and
    /// rendering cannot disagree about the title.
    fn ensure_manifest_current(&self) -> Result<(), ClewError> {
        let current: Manifest = read(&self.path("codeclew-docs.yaml")?, MAX_RECORD)?;
        if current != self.manifest {
            return Err(ClewError::new(
                ErrorCode::WwConflict,
                "documentation manifest changed since this repository was opened; reopen the repository",
            ));
        }
        Ok(())
    }

    pub fn put<T: Serialize>(
        &self,
        path: &str,
        value: &T,
        expected: Option<&str>,
    ) -> Result<Value, ClewError> {
        let _lock = self.lock()?;
        let current = self.input_digest()?;
        let data = bytes(value)?;
        let target = self.path(path)?;
        let same = target.exists() && fs::read(&target).map_err(io_error)? == data;
        if same {
            return Ok(
                json!({"schema":"codeclew-docs-write/1.0","status":"CURRENT","inputDigest":current}),
            );
        }
        if expected != Some(current.as_str()) {
            return Err(ClewError::new(
                ErrorCode::WwConflict,
                "documentation input digest changed; reload the catalogue and retry with its current inputDigest",
            ));
        }
        self.atomic(path, &data)?;
        Ok(
            json!({"schema":"codeclew-docs-write/1.0","status":"SAVED","inputDigest":self.input_digest()?}),
        )
    }

    pub fn service_add(&self, value: Service, expected: Option<&str>) -> Result<Value, ClewError> {
        validate_service(&value)?;
        let services = self.services()?;
        if let Some(old) = services.get(&value.id)
            && old.repository_id != value.repository_id
        {
            return Err(invalid(
                "repository identity migration requires an explicit new service record",
            ));
        }
        let mut result = self.put(
            &format!("catalog/services/{}.json", value.id),
            &value,
            expected,
        )?;
        result["sections"] = json!(super::sections::records(&value.id, None));
        Ok(result)
    }
    pub fn interaction_put(
        &self,
        value: Interaction,
        expected: Option<&str>,
    ) -> Result<Value, ClewError> {
        validate_interaction(&value, &self.services()?)?;
        self.put(
            &format!("catalog/interactions/{}.json", value.id),
            &value,
            expected,
        )
    }
    pub fn interaction_remove(&self, id: &str, expected: &str) -> Result<Value, ClewError> {
        if !valid_id(id) {
            return Err(invalid("invalid interaction ID"));
        }
        let _lock = self.lock()?;
        if self.input_digest()? != expected {
            return Err(ClewError::new(
                ErrorCode::WwConflict,
                "documentation input digest changed",
            ));
        }
        if self
            .scenarios()?
            .values()
            .any(|s| s.interactions.iter().any(|i| i == id))
        {
            return Err(invalid(
                "interaction is referenced by a scenario; update that scenario first",
            ));
        }
        let path = self.path(&format!("catalog/interactions/{id}.json"))?;
        if path.exists() {
            fs::remove_file(path).map_err(io_error)?;
        }
        Ok(
            json!({"schema":"codeclew-docs-write/1.0","status":"REMOVED","inputDigest":self.input_digest()?}),
        )
    }
}

fn validate_manifest_schema(manifest: &Manifest) -> Result<(), ClewError> {
    if manifest.schema != format!("codeclew-documentation/{VERSION}") {
        return Err(invalid(
            "DOCS_REINDEX_REQUIRED: unsupported documentation root schema; initialize a fresh documentation root and reindex its sources",
        ));
    }
    Ok(())
}

pub fn validate_service(s: &Service) -> Result<(), ClewError> {
    super::modules::validate(s)?;
    if s.schema != "codeclew-documentation-service/1.0"
        || !valid_id(&s.id)
        || !valid_id(&s.repository_id)
        || s.title.trim().is_empty()
        || !safe_url(&s.repository)
        || s.target_ref.is_empty()
        || s.target_ref.starts_with('-')
    {
        return Err(invalid(
            "invalid service identity, credential-free repository URL, or revision selector",
        ));
    }
    let supported = match s.language.as_str() {
        "java" => matches!(
            s.profile.as_str(),
            "java-17plus-maven-read-only"
                | "java-17plus-gradle-read-only"
                | "java-17plus-maven-writable-then-seal"
        ),
        "kotlin" => matches!(
            s.profile.as_str(),
            "kotlin-jvm-maven-analysis"
                | "kotlin-jvm-gradle-analysis"
                | "kotlin-2.3.0-maven-single"
                | "kotlin-2.4.0-gradle-single"
                | "kotlin-2.4.10-gradle-single"
        ),
        _ => false,
    };
    let source_profile =
        s.profile == "source-syntax" && matches!(s.language.as_str(), "python" | "java" | "kotlin");
    if source_profile {
        let config = s
            .source
            .as_ref()
            .ok_or_else(|| invalid("source-syntax requires source roots and dialect"))?;
        if config.roots.is_empty() || config.roots.len() > 64 || config.dialect.trim().is_empty() {
            return Err(invalid(
                "source roots and dialect must be bounded and nonempty",
            ));
        }
        for root in &config.roots {
            if root != "." {
                relative(root)?;
            }
        }
    } else if s.source.is_some() {
        return Err(invalid(
            "source configuration requires the source-syntax profile",
        ));
    }
    if !source_profile {
        if !supported {
            return Err(ClewError::new(
                ErrorCode::UnsupportedLanguage,
                "durable documentation requires a Java 17+ or Kotlin/JVM 1.9+ Maven/Gradle analysis profile",
            ));
        }
        // Explicit compilation selection: duplicates and empty selectors are
        // rejected so one scope cannot multiply work or create ambiguous
        // authority.
        let compilations = s.effective_compilations();
        if compilations.is_empty() {
            return Err(invalid(
                "service requires at least one compilation selector",
            ));
        }
        if compilations.len() > crate::limits::MAX_SELECTED_COMPILATIONS {
            return Err(invalid(format!(
                "at most {} compilation selectors may be selected",
                crate::limits::MAX_SELECTED_COMPILATIONS
            )));
        }
        let mut seen = std::collections::BTreeSet::new();
        for selector in &compilations {
            if selector.is_empty() {
                return Err(invalid("a compilation selector must not be empty"));
            }
            if !seen.insert(selector) {
                return Err(invalid("a compilation selector must not be duplicated"));
            }
        }
    }
    if s.contract_files.len() > 128 {
        return Err(invalid("at most 128 contract files may be selected"));
    }
    for file in &s.contract_files {
        relative(file)?;
    }
    if s.annotation_processor_paths.len() > 32 {
        return Err(invalid(
            "at most 32 annotation processor paths may be selected",
        ));
    }
    for coordinate in &s.annotation_processor_paths {
        let parts = coordinate.split(':').collect::<Vec<_>>();
        if parts.len() != 3
            || parts.iter().any(|part| part.is_empty())
            || coordinate.contains('/')
            || coordinate.contains('\\')
        {
            return Err(invalid(
                "annotation processor path must be a Maven coordinate group:artifact:version",
            ));
        }
    }
    if let Some(template) = &s.source_link_template
        && (!safe_url(template)
            || !["{revision}", "{file}"]
                .iter()
                .all(|token| template.contains(token)))
    {
        return Err(invalid(
            "source link template requires HTTPS, {revision} and {file}, without credentials or query strings",
        ));
    }
    Ok(())
}

pub fn endpoint(e: &Endpoint, services: &BTreeMap<String, Service>) -> Result<(), ClewError> {
    if !services.contains_key(&e.service) {
        return Err(invalid("endpoint refers to an unregistered service"));
    }
    if let Some(s) = &e.selector
        && (s.language != services[&e.service].language
            || s.owner.is_empty()
            || s.name.is_empty()
            || s.scope
                .as_ref()
                .is_some_and(|scope| scope.len() > 1024 || scope.chars().any(char::is_control))
            || s.parameter_types.as_ref().is_some_and(|p| p.len() > 64))
    {
        return Err(invalid("endpoint selector is incomplete or unsupported"));
    }
    if e.call_site
        .as_ref()
        .is_some_and(|c| c.target.is_empty() || c.ordinal.is_some_and(|i| i > 4096))
    {
        return Err(invalid("invalid call-site selector"));
    }
    Ok(())
}

fn validate_interaction(
    i: &Interaction,
    services: &BTreeMap<String, Service>,
) -> Result<(), ClewError> {
    if i.schema != "codeclew-documentation-interaction/1.0"
        || !valid_id(&i.id)
        || i.title.trim().is_empty()
        || !matches!(
            i.declaration.origin.as_str(),
            "human" | "agent-proposal" | "imported"
        )
        || i.declaration.rationale.trim().is_empty()
        || !matches!(i.transport.kind.as_str(), "http" | "kafka")
    {
        return Err(invalid(
            "invalid interaction identity, declaration origin, or unsupported transport",
        ));
    }
    endpoint(&i.from, services)?;
    endpoint(&i.to, services)?;
    if i.from.service == i.to.service {
        return Err(invalid("an interaction must connect two distinct services"));
    }
    if (i.transport.kind == "kafka"
        && (i.transport.method.is_some()
            || i.transport.path.is_some()
            || i.transport.topic.as_ref().is_none_or(|topic| {
                topic.is_empty() || topic.len() > 249 || topic.contains(['\n', '\r'])
            })))
        || (i.transport.kind == "http" && i.transport.topic.is_some())
    {
        return Err(invalid(
            "Kafka interactions require a bounded topic and no HTTP fields; HTTP interactions have no topic",
        ));
    }
    if i.transport.method.as_ref().is_some_and(|m| {
        !matches!(
            m.as_str(),
            "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" | "OPTIONS"
        )
    }) || i
        .transport
        .path
        .as_ref()
        .is_some_and(|p| !p.starts_with('/') || p.contains(['\n', '\r']))
    {
        return Err(invalid("invalid HTTP method/path declaration"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn service(id: &str) -> Service {
        Service {
            schema: "codeclew-documentation-service/1.0".into(),
            id: id.into(),
            title: id.into(),
            repository_id: id.into(),
            repository: format!("https://example.invalid/{id}"),
            language: "java".into(),
            profile: "java-17plus-maven-read-only".into(),
            source: None,
            modules: None,
            compilations: vec![":/main".into()],
            target_ref: "main".into(),
            source_link_template: None,
            contract_files: vec![],
            annotation_processor_paths: vec![],
        }
    }
    fn scenario(id: &str, service_id: &str) -> Scenario {
        serde_json::from_value(serde_json::json!({
            "schema":"codeclew-documentation-process/1.0",
            "id":id,"title":id,"summary":"A bounded process declaration",
            "root":{"service":service_id},"interactions":[],"maxDepth":4,"maxNodes":64,
            "process":{"scope":id,"participants":[service_id],"objects":[],
                "trigger":"request","outcomes":["complete"],"linkedSubviews":[]}
        }))
        .unwrap()
    }
    fn setup() -> (tempfile::TempDir, Repository) {
        let t = tempfile::tempdir().unwrap();
        Repository::init(t.path(), "Architecture").unwrap();
        let r = Repository::open(t.path()).unwrap();
        (t, r)
    }
    #[test]
    fn records_skip_only_actual_state_sidecars_and_preserve_suffix_ids() {
        let (_t, r) = setup();
        let dir = r.root.join("scenarios");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("task.yaml"), "kind: record").unwrap();
        fs::write(
            dir.join("task-states.yaml"),
            "schema: codeclew-documentation-process-states/1.0",
        )
        .unwrap();
        fs::write(
            dir.join("order-states.yaml"),
            r#"{"schema":"ordinary-scenario","id":"order-states"}"#,
        )
        .unwrap();
        let rows: BTreeMap<String, serde_json::Value> = r.records("scenarios", "yaml").unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.contains_key("task"));
        assert!(rows.contains_key("order-states"));
        assert!(!rows.contains_key("task-states"));

        let services = r.root.join("catalog/services");
        fs::create_dir_all(&services).unwrap();
        fs::write(
            services.join("orders-states.json"),
            r#"{"id":"orders-states"}"#,
        )
        .unwrap();
        let rows: BTreeMap<String, serde_json::Value> =
            r.records("catalog/services", "json").unwrap();
        assert!(rows.contains_key("orders-states"));
    }

    #[test]
    fn empty_process_state_field_is_omitted_from_legacy_input_serialization() {
        let (_t, r) = setup();
        let inputs = r.inputs().unwrap();
        let encoded = crate::canonical::bytes(&inputs).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
        assert!(value.get("processStates").is_none());
    }

    #[test]
    fn repository_inputs_capture_and_validate_belonging_state_sidecars() {
        let (_t, r) = setup();
        r.service_add(service("orders"), Some(&r.input_digest().unwrap()))
            .unwrap();
        r.atomic(
            "scenarios/checkout.yaml",
            &crate::canonical::bytes(&scenario("checkout", "orders")).unwrap(),
        )
        .unwrap();
        // A scenario named `order-states` is an ordinary process declaration,
        // even when `order.yaml` also exists. Its own sidecar has the doubled
        // suffix `order-states-states.yaml`.
        for id in ["order", "order-states"] {
            r.atomic(
                &format!("scenarios/{id}.yaml"),
                &crate::canonical::bytes(&scenario(id, "orders")).unwrap(),
            )
            .unwrap();
        }
        let order_states = "schema: codeclew-documentation-process-states/1.0\nid: order-states\ntitle: Order lifecycle\ninitial: NEW\nfinal: [DONE]\nstates:\n  NEW: waiting\n  DONE: finished\ntransitions: []\n";
        r.atomic(
            "scenarios/order-states-states.yaml",
            order_states.as_bytes(),
        )
        .unwrap();
        let before = r.input_digest().unwrap();
        let valid = "schema: codeclew-documentation-process-states/1.0\nid: checkout\ntitle: Checkout lifecycle\ninitial: NEW\nfinal: [DONE]\nstates:\n  NEW: waiting\n  DONE: finished\ntransitions: []\n";
        r.atomic("scenarios/checkout-states.yaml", valid.as_bytes())
            .unwrap();
        let inputs = r.inputs().unwrap();
        assert_eq!(inputs.process_states["checkout"].initial, "NEW");
        assert!(inputs.scenarios.contains_key("order"));
        assert!(inputs.scenarios.contains_key("order-states"));
        assert!(!inputs.process_states.contains_key("order"));
        assert_eq!(inputs.process_states["order-states"].initial, "NEW");
        assert_ne!(digest(&inputs).unwrap(), before);

        let invalid_sidecar = valid.replace("initial: NEW", "initial: UNDECLARED");
        r.atomic("scenarios/checkout-states.yaml", invalid_sidecar.as_bytes())
            .unwrap();
        assert!(r.inputs().is_err());
    }
    #[test]
    fn current_git_clone_initializes_local_state_and_binds_without_changing_declarations() {
        let temporary = tempfile::tempdir().unwrap();
        let source = temporary.path().join("source");
        let original = temporary.path().join("docs-original");
        let cloned = temporary.path().join("docs-clone");
        fs::create_dir_all(source.join("src")).unwrap();
        let git = |path: &Path, args: &[&str]| {
            let output = std::process::Command::new("git")
                .current_dir(path)
                .args([
                    "-c",
                    "user.name=Fixture",
                    "-c",
                    "user.email=fixture@example.invalid",
                ])
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        git(&source, &["init", "--initial-branch=main"]);
        git(
            &source,
            &["remote", "add", "origin", "https://example.invalid/orders"],
        );
        fs::write(
            source.join("src/Worker.java"),
            "class Worker { int value() { return 1; } }",
        )
        .unwrap();
        git(&source, &["add", "."]);
        git(&source, &["commit", "-m", "Fixture"]);
        Repository::init(&original, "Architecture").unwrap();
        let repo = Repository::open(&original).unwrap();
        let mut selected = service("orders");
        selected.profile = "source-syntax".into();
        selected.compilations.clear();
        selected.source = Some(SourceConfig {
            roots: vec!["src".into()],
            dialect: "17".into(),
        });
        selected.target_ref = "main".into();
        repo.service_add(selected, Some(&repo.input_digest().unwrap()))
            .unwrap();
        fs::write(original.join("manual.md"), "Preserve authored guidance.\n").unwrap();
        let original_digest = repo.input_digest().unwrap();
        git(&original, &["init", "--initial-branch=main"]);
        git(&original, &["add", "."]);
        git(&original, &["commit", "-m", "Current documentation"]);
        git(
            temporary.path(),
            &[
                "clone",
                original.to_str().unwrap(),
                cloned.to_str().unwrap(),
            ],
        );
        assert!(!cloned.join(".codeclew").exists());
        assert!(
            Repository::open(&cloned)
                .unwrap_err()
                .message
                .contains("docs init")
        );
        let declaration_bytes = fs::read(cloned.join("catalog/services/orders.json")).unwrap();
        Repository::init(&cloned, "Architecture").unwrap();
        let clone = Repository::open(&cloned).unwrap();
        assert_eq!(clone.input_digest().unwrap(), original_digest);
        assert_eq!(
            fs::read(cloned.join("catalog/services/orders.json")).unwrap(),
            declaration_bytes
        );
        assert_eq!(
            fs::read_to_string(cloned.join("manual.md")).unwrap(),
            "Preserve authored guidance.\n"
        );
        super::super::analysis::bind(&clone, "orders", &source).unwrap();
    }

    #[test]
    fn initialization_rejects_obsolete_and_partial_roots_without_resetting_them() {
        let old = tempfile::tempdir().unwrap();
        let bytes = b"schema: codeclew-documentation/1.0\ntitle: Architecture\n";
        fs::write(old.path().join("codeclew-docs.yaml"), bytes).unwrap();
        for result in [
            Repository::init(old.path(), "Architecture").map(|_| ()),
            Repository::open(old.path()).map(|_| ()),
        ] {
            assert!(
                result
                    .unwrap_err()
                    .message
                    .contains("DOCS_REINDEX_REQUIRED")
            );
        }
        assert_eq!(
            fs::read(old.path().join("codeclew-docs.yaml")).unwrap(),
            bytes
        );
        assert!(!old.path().join(".codeclew").exists());
        let (root, repo) = setup();
        let marker = repo.path(".codeclew/cache/object-layout.json").unwrap();
        let layout: Value = serde_json::from_slice(&fs::read(&marker).unwrap()).unwrap();
        let database = repo.path(layout["database"].as_str().unwrap()).unwrap();
        drop(repo);
        fs::remove_file(&database).unwrap();
        assert_eq!(
            Repository::init(root.path(), "Architecture")
                .unwrap_err()
                .code,
            ErrorCode::StateCorrupt
        );
        assert!(!database.exists());
        fs::remove_file(&marker).unwrap();
        let retained = root.path().join(".codeclew/cache/retained-data");
        fs::write(&retained, b"keep").unwrap();
        assert!(
            Repository::init(root.path(), "Architecture")
                .unwrap_err()
                .message
                .contains("DOCS_REINDEX_REQUIRED")
        );
        assert_eq!(fs::read(retained).unwrap(), b"keep");
        assert!(!marker.exists());
    }

    #[test]
    fn records_survive_reopen_and_detect_concurrent_writes() {
        let (t, r) = setup();
        let first = r.input_digest().unwrap();
        let s = service("orders");
        r.service_add(s.clone(), Some(&first)).unwrap();
        assert!(r.service_add(service("inventory"), Some(&first)).is_err());
        assert_eq!(r.service_add(s, Some(&first)).unwrap()["status"], "CURRENT");
        let reopened = Repository::open(t.path()).unwrap();
        assert_eq!(reopened.services().unwrap().len(), 1);
    }
    #[test]
    fn detects_concurrent_manifest_edit_but_accepts_semantically_equal_yaml() {
        let (t, r) = setup();
        let before = r.input_digest().unwrap();
        let manifest_path = r.root.join("codeclew-docs.yaml");

        fs::write(
            &manifest_path,
            "# Formatting and comments are not part of manifest identity.\n\ntitle: Architecture\nschema: codeclew-documentation/2.0\n",
        )
        .unwrap();
        assert_eq!(r.input_digest().unwrap(), before);

        fs::write(
            &manifest_path,
            "schema: codeclew-documentation/2.0\ntitle: Changed title\n",
        )
        .unwrap();
        let error = r.input_digest().unwrap_err();
        assert_eq!(error.code, ErrorCode::WwConflict);
        assert!(error.message.contains("reopen the repository"));
        let write_error = r.service_add(service("orders"), Some(&before)).unwrap_err();
        assert_eq!(write_error.code, ErrorCode::WwConflict);
        assert!(!r.root.join("catalog/services/orders.json").exists());

        let reopened = Repository::open(t.path()).unwrap();
        let after = reopened.input_digest().unwrap();
        assert_ne!(after, before);
        reopened
            .service_add(service("orders"), Some(&after))
            .unwrap();
        assert!(reopened.root.join("catalog/services/orders.json").exists());
    }
    #[test]
    fn compilation_limit_service_accepts_128_and_rejects_129_selectors() {
        let (t, r) = setup();
        let mut s = service("sales");
        let selectors = (0..128)
            .map(|index| format!(":m{index}/main"))
            .collect::<Vec<_>>();
        s.compilations = selectors.clone();
        assert_eq!(s.compilations.len(), 128);
        r.service_add(s.clone(), Some(&r.input_digest().unwrap()))
            .unwrap();

        let mut over = s;
        over.compilations = (0..129)
            .map(|index| format!(":m{index}/main"))
            .collect::<Vec<_>>();
        assert!(super::validate_service(&over).is_err());
        assert!(
            Repository::open(t.path())
                .unwrap()
                .services()
                .unwrap()
                .iter()
                .all(|x| x.1.id != "over")
        );
    }
    #[test]
    fn rejects_unknown_fields_and_dangling_references() {
        let (_t, r) = setup();
        let input = r.input_digest().unwrap();
        r.service_add(service("orders"), Some(&input)).unwrap();
        let i = json!({"schema":"codeclew-documentation-interaction/1.0","id":"reserve","title":"Reserve","from":{"service":"orders"},"to":{"service":"missing"},"transport":{"kind":"http"},"declaration":{"origin":"human","rationale":"Engineer declaration"}});
        assert!(
            r.interaction_put(
                serde_json::from_value(i.clone()).unwrap(),
                Some(&r.input_digest().unwrap())
            )
            .is_err()
        );
        let mut bad = i;
        bad["unexpected"] = json!(true);
        assert!(serde_json::from_value::<Interaction>(bad).is_err());
    }
    #[test]
    fn preserves_manual_prose_and_refuses_escaping_paths() {
        let (_t, r) = setup();
        fs::write(r.root.join("AGENTS.md"), "Existing instructions\n").unwrap();
        fs::write(r.root.join("docs/manual.md"), "Human prose\n").unwrap();
        Repository::init(&r.root, "Architecture").unwrap();
        assert_eq!(
            fs::read_to_string(r.root.join("AGENTS.md")).unwrap(),
            "Existing instructions\n"
        );
        assert_eq!(
            fs::read_to_string(r.root.join("docs/manual.md")).unwrap(),
            "Human prose\n"
        );
        for path in [
            "../outside",
            "/tmp/outside",
            "docs/../../outside",
            "docs/link:evil",
        ] {
            assert!(r.path(path).is_err());
        }
    }
    #[cfg(unix)]
    #[test]
    fn rejects_symlinked_output_and_writer_overlap() {
        let (t, r) = setup();
        std::os::unix::fs::symlink(t.path(), r.root.join("docs/link")).unwrap();
        assert!(r.path("docs/link/escape").is_err());
        let _lock = r.lock().unwrap();
        assert!(r.lock().is_err());
    }

    #[test]
    fn per_service_annotation_processor_paths_are_validated() {
        let mut s = service("orders");
        s.annotation_processor_paths = vec!["org.projectlombok:lombok:1.18.22".into()];
        assert!(validate_service(&s).is_ok());

        // Malformed or oversized selections are rejected.
        for bad in [
            "org.projectlombok",
            "org.projectlombok:lombok",
            "org.projectlombok:lombok:",
            "org.projectlombok:lombok:1.18.22:extra",
            "org/projectlombok:lombok:1.18.22",
        ] {
            let mut s = service("orders");
            s.annotation_processor_paths = vec![bad.into()];
            let error = validate_service(&s).unwrap_err();
            assert_eq!(error.code, ErrorCode::InvalidInput, "{bad}: {error}");
        }

        let mut s = service("orders");
        s.annotation_processor_paths = (0..33).map(|i| format!("org.example:p{i}:1.0")).collect();
        let error = validate_service(&s).unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidInput);
    }
}
