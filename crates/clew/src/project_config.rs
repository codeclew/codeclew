//! Caller-local launch preferences, distinct from the selected source snapshot.
use crate::error::{ClewError, ErrorCode};
use crate::maven::MavenSettings;
use serde::Deserialize;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

const MAX_CONFIG_BYTES: u64 = 64 * 1024;

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ProjectConfig {
    version: Option<u32>,
    maven: MavenConfig,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct MavenConfig {
    settings: Option<PathBuf>,
}

pub fn maven_settings(
    repository: &Path,
    explicit: Option<&Path>,
) -> Result<Option<MavenSettings>, ClewError> {
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
                invalid(format!("Invalid codeclew.yaml{position}; expected optional version: 1 and maven.settings: <path>. Unknown keys and duplicate fields are rejected."))
            })?;
            if config.version.is_some_and(|version| version != 1) {
                return Err(invalid(
                    "Unsupported codeclew.yaml version; expected version: 1",
                ));
            }
            config
        }
    };
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
}
