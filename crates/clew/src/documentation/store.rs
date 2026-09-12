//! Separate machine-owned records preserve authored YAML comments and manual prose.
use super::{bytes, digest, invalid, io_error, model::*};
use crate::error::{ClewError, ErrorCode};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};

pub const MAX_RECORD: u64 = 2 * 1024 * 1024;
pub const MAX_RECORDS: usize = 1024;

#[derive(Debug)]
pub struct Repository {
    pub root: PathBuf,
    pub manifest: Manifest,
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
        let repo = Self { root, manifest };
        let _lock = repo.lock()?;
        let path = repo.path("codeclew-docs.yaml")?;
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
        let instructions = "# Documentation ownership\n\nRead codeclew-docs.yaml and catalog records first. Use clew docs commands for\nvalidated operations. Bind relocated checkouts with clew docs bind.\nManual notes belong outside docs/generated and narratives; never overwrite them.\nDefault service scope covers every discovered entrypoint, including explicit gaps.\nUse narrative 1.1 with domain explanation paragraphs linked to every diagram step.\nExplain business inputs, checks, state changes, failures and outcomes from source.\nPreserve callback scheduling and asynchronous message boundaries.\nDeclared interactions are not compiler or runtime proof. Run clew docs check\nbefore refreshing; preserve stable declaration and scenario IDs.\n";
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
        Ok(
            json!({"schema":"codeclew-docs-init/1.0", "status":"READY", "inputDigest":repo.input_digest()?}),
        )
    }

    pub fn open(root: &Path) -> Result<Self, ClewError> {
        let root = root.canonicalize().map_err(io_error)?;
        let provisional = Self {
            root,
            manifest: Manifest {
                schema: String::new(),
                title: String::new(),
            },
        };
        let manifest: Manifest = read(&provisional.path("codeclew-docs.yaml")?, MAX_RECORD)?;
        if manifest.schema != format!("codeclew-documentation/{VERSION}") {
            return Err(invalid("unsupported documentation schema"));
        }
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

    fn records<T: DeserializeOwned>(
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
            result.insert(id, read(&entry.path(), MAX_RECORD)?);
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
            if value.schema != "codeclew-documentation-scenario/1.0"
                || id != &value.id
                || !valid_id(id)
                || value.max_depth > 16
                || value.max_nodes == 0
                || value.max_nodes > 512
            {
                return Err(invalid("invalid scenario identity or traversal bounds"));
            }
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
        // Parse independently authored files without rewriting their bytes/comments.
        digest(
            &json!({"manifest":self.manifest,"services":self.services()?,"interactions":self.interactions()?,"scenarios":self.scenarios()?}),
        )
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
        self.put(
            &format!("catalog/services/{}.json", value.id),
            &value,
            expected,
        )
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
            "java-17plus-maven-read-only" | "java-17plus-gradle-read-only"
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
        if let Some(semantic) = &config.semantic {
            let mut provider = s.clone();
            provider.source = None;
            provider.profile = semantic.profile.clone();
            provider.compilation = semantic.compilation.clone();
            validate_service(&provider)?;
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
    if !source_profile && (!supported || s.compilation.is_empty()) {
        return Err(ClewError::new(
            ErrorCode::UnsupportedLanguage,
            "durable documentation requires a Java 17+ or Kotlin/JVM 1.9+ Maven/Gradle analysis profile",
        ));
    }
    if s.contract_files.len() > 128 {
        return Err(invalid("at most 128 contract files may be selected"));
    }
    for file in &s.contract_files {
        relative(file)?;
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
            compilation: ":/main".into(),
            target_ref: "main".into(),
            source_link_template: None,
            contract_files: vec![],
        }
    }
    fn setup() -> (tempfile::TempDir, Repository) {
        let t = tempfile::tempdir().unwrap();
        Repository::init(t.path(), "Architecture").unwrap();
        let r = Repository::open(t.path()).unwrap();
        (t, r)
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
}
