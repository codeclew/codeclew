//! Caller-local launch preferences, distinct from the selected source snapshot.
use crate::error::{ClewError, ErrorCode};
use crate::maven::MavenSettings;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::ffi::OsStr;
#[cfg(unix)]
use std::os::fd::AsRawFd;
use uuid::Uuid;

use crate::{canonical, state::StateAuthority};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

const MAX_CONFIG_BYTES: u64 = 64 * 1024;

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ProjectConfig {
    version: Option<u32>,
    maven: MavenConfig,
    gradle: GradleConfig,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct MavenConfig {
    settings: Option<PathBuf>,
}

const MAX_PROPERTIES: usize = 32;

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct GradleConfig {
    #[serde(deserialize_with = "project_props")]
    project_props: BTreeMap<String, String>,
}

fn project_props<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<BTreeMap<String, String>, D::Error> {
    use serde::de::Error;
    use serde_yaml_ng::Value;
    let Value::Mapping(map) = Value::deserialize(deserializer)? else {
        return Err(D::Error::custom(
            "expected a map of string project properties",
        ));
    };
    if map.len() > MAX_PROPERTIES {
        return Err(D::Error::custom("too many project properties"));
    }
    map.into_iter()
        .map(|(key, value)| match (key, value) {
            (Value::String(key), Value::String(value))
                if project_property_key(&key)
                    && value.len() <= 1024
                    && !value.chars().any(char::is_control) =>
            {
                Ok((key, value))
            }
            _ => Err(D::Error::custom(
                "invalid project property key or string value",
            )),
        })
        .collect()
}

fn project_property_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    key.len() <= 128
        && key
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && key
            .bytes()
            .all(|ch| ch.is_ascii_alphanumeric() || b"_.-".contains(&ch))
        && ![
            "codeclew",
            "semantic.thread",
            "org.gradle",
            "systemprop",
            "kotlin.project.persistent.dir",
        ]
        .iter()
        .any(|prefix| lower == *prefix || lower.starts_with(&format!("{prefix}.")))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GradlePropertyBinding {
    pub id: Uuid,
    pub count: usize,
}

impl GradlePropertyBinding {
    pub(crate) fn is_valid(&self) -> bool {
        self.id.get_version_num() == 4 && (1..=MAX_PROPERTIES).contains(&self.count)
    }

    pub(crate) fn summary(&self) -> serde_json::Value {
        gradle_property_summary(self.count)
    }
}

fn gradle_property_summary(count: usize) -> serde_json::Value {
    serde_json::json!({"source":"REPOSITORY_CONFIG","status":"APPLIED","count":count})
}

pub(crate) fn safe_gradle_property_summary(value: &serde_json::Value) -> Option<serde_json::Value> {
    let count = usize::try_from(value.get("count")?.as_u64()?).ok()?;
    let binding = value
        .get("inputBinding")?
        .as_str()?
        .strip_prefix("sha256:")?;
    ((1..=MAX_PROPERTIES).contains(&count)
        && value.get("source")?.as_str()? == "REPOSITORY_CONFIG"
        && value.get("status")?.as_str()? == "APPLIED"
        && binding.len() == 64
        && binding.bytes().all(|byte| byte.is_ascii_hexdigit()))
    .then(|| gradle_property_summary(count))
}

// Values are intentionally neither Debug nor Serialize. Only the private
// worker launch transport receives the arguments.
pub(crate) struct GradleProperties {
    pub binding: GradlePropertyBinding,
    values: BTreeMap<String, String>,
}

impl GradleProperties {
    pub(crate) fn args(&self) -> impl Iterator<Item = String> + '_ {
        self.values
            .iter()
            .map(|(key, value)| format!("-P{key}={value}"))
    }
}

pub(crate) fn gradle_properties(
    repo: &Path,
    state: &StateAuthority,
    repository_key: &str,
) -> Result<Option<GradleProperties>, ClewError> {
    let values = read_config(repo)?.gradle.project_props;
    if values.is_empty() {
        return Ok(None);
    }
    // Keep the lookup digest private. A random opaque identifier is all that
    // can leave this store; it does not allow guessing low-entropy values.
    let fingerprint =
        canonical::hash(&(&repository_key, &values)).map_err(|_| property_state_error())?;
    let dir = state
        .directory(Path::new("gradle-property-identities"))
        .map_err(|_| property_state_error())?;
    let name = fingerprint.trim_start_matches("sha256:");
    let lock = state
        .directory(Path::new("locks"))
        .map_err(|_| property_state_error())?
        .open_lock(OsStr::new("gradle-property-identities.lock"))
        .map_err(|_| property_state_error())?;
    #[cfg(unix)]
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) } != 0 {
        return Err(property_state_error());
    }
    let path = dir
        .resolved_path()
        .map_err(|_| property_state_error())?
        .join(name);
    let id = if state
        .private_file_exists(&path)
        .map_err(|_| property_state_error())?
    {
        let bytes = state
            .read_private_file(&path, 36)
            .map_err(|_| property_state_error())?;
        Uuid::try_parse_ascii(&bytes).map_err(|_| property_state_error())?
    } else {
        let id = Uuid::new_v4();
        state
            .write_private_atomic(&path, id.to_string().as_bytes())
            .map_err(|_| property_state_error())?;
        id
    };
    let binding = GradlePropertyBinding {
        id,
        count: values.len(),
    };
    if !binding.is_valid() {
        return Err(property_state_error());
    }
    Ok(Some(GradleProperties { binding, values }))
}

fn property_state_error() -> ClewError {
    ClewError::new(
        ErrorCode::StateCorrupt,
        "Managed Gradle property identity is unavailable",
    )
}

fn read_config(repository: &Path) -> Result<ProjectConfig, ClewError> {
    let path = repository.join("codeclew.yaml");
    let config = match File::open(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => ProjectConfig::default(),
        Err(_) => return Err(invalid("codeclew.yaml is not readable")),
        Ok(file) => {
            if !file
                .metadata()
                .is_ok_and(|metadata| metadata.is_file() && metadata.len() <= MAX_CONFIG_BYTES)
            {
                return Err(invalid(
                    "codeclew.yaml must be a regular file of at most 64 KiB",
                ));
            }
            let mut bytes = Vec::new();
            file.take(MAX_CONFIG_BYTES + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| invalid("codeclew.yaml is not readable"))?;
            if bytes.len() as u64 > MAX_CONFIG_BYTES {
                return Err(invalid("codeclew.yaml exceeds 64 KiB"));
            }
            let config: ProjectConfig = serde_yaml_ng::from_slice(&bytes).map_err(|error| {
                let position = error.location().map(|location| format!(" at line {}, column {}", location.line(), location.column())).unwrap_or_default();
                invalid(format!("Invalid codeclew.yaml{position}; expected optional version: 1, maven.settings, and gradle.projectProps with validated string keys and values. Unknown and duplicate keys are rejected."))
            })?;
            if config.version.is_some_and(|version| version != 1) {
                return Err(invalid(
                    "Unsupported codeclew.yaml version; expected version: 1",
                ));
            }
            config
        }
    };
    Ok(config)
}

pub fn maven_settings(
    repository: &Path,
    explicit: Option<&Path>,
) -> Result<Option<MavenSettings>, ClewError> {
    let config = read_config(repository)?;
    if let Some(path) = explicit {
        return MavenSettings::capture(path).map(Some);
    }
    config
        .maven
        .settings
        .map(|path| {
            if path.as_os_str().is_empty() {
                return Err(invalid("maven.settings must name a settings.xml file"));
            }
            MavenSettings::capture(&repository.join(path))
        })
        .transpose()
}

fn invalid(message: impl Into<String>) -> ClewError {
    ClewError::new(ErrorCode::InvalidInput, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn local_settings_are_relative_to_repository_and_cli_overrides_them() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("settings with spaces.xml"), "<settings/>").unwrap();
        fs::write(
            root.path().join("codeclew.yaml"),
            "version: 1\nmaven:\n  settings: 'settings with spaces.xml'\n",
        )
        .unwrap();
        let selected = maven_settings(root.path(), None).unwrap().unwrap();
        assert_eq!(
            selected.path,
            root.path()
                .canonicalize()
                .unwrap()
                .join("settings with spaces.xml")
        );
        let override_file = root.path().join("override.xml");
        fs::write(
            &override_file,
            "<settings><offline>true</offline></settings>",
        )
        .unwrap();
        let selected_override = maven_settings(root.path(), Some(&override_file))
            .unwrap()
            .unwrap();
        assert_ne!(selected.digest, selected_override.digest);
        assert_eq!(
            selected_override.path,
            override_file.canonicalize().unwrap()
        );
        fs::remove_file(root.path().join("codeclew.yaml")).unwrap();
        assert!(maven_settings(root.path(), None).unwrap().is_none());
    }

    #[test]
    fn malformed_or_unknown_config_is_not_silently_ignored_or_echoed() {
        let root = tempfile::tempdir().unwrap();
        for source in [
            "version: 2",
            "maven:\n  secret-token: private-value",
            "maven: [",
            "maven:\n  settings: one\n  settings: two",
        ] {
            fs::write(root.path().join("codeclew.yaml"), source).unwrap();
            let error = maven_settings(root.path(), None).unwrap_err();
            assert_eq!(error.code, ErrorCode::InvalidInput);
            assert!(!error.message.contains("private-value"));
            assert!(!error.message.contains("secret-token"));
        }
    }
    #[test]
    fn gradle_properties_bind_normalized_strings_without_exposing_them() {
        let temp = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(temp.path().join("v2")).unwrap();
        let repo = temp.path();
        let config = repo.join("codeclew.yaml");
        let read = || gradle_properties(repo, &state, "owned-repository").unwrap();
        assert!(read().is_none());
        fs::write(&config, "gradle:\n  projectProps:\n    version: '1.1'\n    env: 'owned spaces ; $() = literal'\n").unwrap();
        let first = read().unwrap();
        assert_eq!(
            first.args().collect::<Vec<_>>(),
            ["-Penv=owned spaces ; $() = literal", "-Pversion=1.1"]
        );
        assert_eq!(
            first.binding.summary(),
            serde_json::json!({"source":"REPOSITORY_CONFIG","status":"APPLIED","count":2})
        );
        fs::write(
            &config,
            "gradle:\n  projectProps: {env: 'owned spaces ; $() = literal', version: '1.1'}\n",
        )
        .unwrap();
        assert_eq!(first.binding, read().unwrap().binding);
        fs::write(
            &config,
            "gradle:\n  projectProps: {env: 'owned spaces ; $() = literal', version: '1.2'}\n",
        )
        .unwrap();
        assert_ne!(first.binding, read().unwrap().binding);
        fs::remove_file(&config).unwrap();
        assert!(read().is_none());
    }

    #[test]
    fn invalid_gradle_properties_are_rejected_without_echoing_values() {
        let temp = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(temp.path().join("v2")).unwrap();
        for map in [
            "{version: 1.1}",
            "{env: true}",
            "{env: null}",
            "{env: [private-marker]}",
            "{env: private-marker, env: another}",
            "{'--init-script': private-marker}",
            "{'x=inject': private-marker}",
            "{'x test': private-marker}",
            "{kotlin.project.persistent.dir: private-marker}",
            "{codeclew.java.compileTask: private-marker}",
            "{semantic.thread.compileTask: private-marker}",
            "{org.gradle.jvmargs: private-marker}",
        ] {
            fs::write(
                temp.path().join("codeclew.yaml"),
                format!("gradle:\n  projectProps: {map}\n"),
            )
            .unwrap();
            let error = gradle_properties(temp.path(), &state, "owned-repository")
                .err()
                .expect(map);
            assert_eq!(error.code, ErrorCode::InvalidInput);
            assert!(!error.message.contains("private-marker"));
        }
    }
}
