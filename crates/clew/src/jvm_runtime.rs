//! The analyzer JVM is independent of the project's Maven/Gradle toolchain.
use crate::canonical;
use crate::error::{ClewError, ErrorCode};
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const WORKER_JAVA_HOME: &str = "CODECLEW_WORKER_JAVA_HOME";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerJvm {
    pub home: PathBuf,
    pub major: u16,
    pub release_digest: String,
    pub selection: String,
}

impl WorkerJvm {
    pub fn select(engine: crate::kotlin_engine::KotlinSemanticEngine) -> Result<Self, ClewError> {
        let required_major = crate::analysis_modules::kotlin_worker_jvm(engine).java_major;
        // An explicit choice must never silently fall through to another JDK.
        if let Some(home) = std::env::var_os(WORKER_JAVA_HOME) {
            return inspect(&PathBuf::from(home), "EXPLICIT_WORKER_JDK", required_major);
        }
        let mut candidates = Vec::new();
        if let Some(home) = std::env::var_os("JAVA_HOME") {
            candidates.push((PathBuf::from(home), "PROJECT_JAVA_HOME"));
        }
        if let Some(path) = std::env::var_os("PATH") {
            for directory in std::env::split_paths(&path) {
                if let Ok(java) = directory.join("java").canonicalize()
                    && let Some(home) = java.parent().and_then(Path::parent)
                {
                    candidates.push((home.to_owned(), "PATH"));
                }
            }
        }
        #[cfg(target_os = "macos")]
        if let Ok(output) = Command::new("/usr/libexec/java_home")
            .args(["-v", &required_major.to_string()])
            .output()
            && output.status.success()
            && let Ok(home) = std::str::from_utf8(&output.stdout)
        {
            candidates.push((PathBuf::from(home.trim()), "SYSTEM_JDK_DISCOVERY"));
        }
        #[cfg(target_os = "linux")]
        for root in ["/usr/lib/jvm", "/usr/java", "/opt/java"] {
            if let Ok(entries) = std::fs::read_dir(root) {
                let mut homes = entries
                    .flatten()
                    .map(|entry| entry.path())
                    .collect::<Vec<_>>();
                homes.sort();
                candidates.extend(homes.into_iter().map(|home| (home, "SYSTEM_JDK_DISCOVERY")));
            }
        }
        let mut seen = BTreeSet::new();
        for (home, selection) in candidates {
            if seen.insert(home.clone())
                && let Ok(jvm) = inspect(&home, selection, required_major)
            {
                return Ok(jvm);
            }
        }
        Err(unavailable(
            "no compatible worker JDK was found",
            required_major,
        ))
    }

    /// The generated launcher uses this variable only for its JVM command.
    /// JAVA_HOME and PATH remain unchanged for project build subprocesses.
    pub fn configure(&self, command: &mut Command) {
        command.env(WORKER_JAVA_HOME, &self.home);
    }
}

fn inspect(home: &Path, selection: &str, required_major: u16) -> Result<WorkerJvm, ClewError> {
    if !home.is_absolute() {
        return Err(unavailable(
            "the selected worker JDK home must be absolute",
            required_major,
        ));
    }
    let home = home.canonicalize().map_err(|_| {
        unavailable(
            "the selected worker JDK home does not exist",
            required_major,
        )
    })?;
    if !home.join("bin/java").is_file() || !home.join("bin/javac").is_file() {
        return Err(unavailable(
            "the selected worker JDK must contain bin/java and bin/javac",
            required_major,
        ));
    }
    let release = std::fs::read_to_string(home.join("release")).map_err(|_| {
        unavailable(
            "the selected worker JDK has no readable release metadata",
            required_major,
        )
    })?;
    let major = release
        .lines()
        .find_map(|line| {
            line.strip_prefix("JAVA_VERSION=")?
                .trim_matches('"')
                .split(['.', '-', '+'])
                .next()?
                .parse::<u16>()
                .ok()
        })
        .ok_or_else(|| {
            unavailable(
                "the selected worker JDK version cannot be determined",
                required_major,
            )
        })?;
    if major != required_major {
        return Err(unavailable(
            &format!("the selected worker JDK is Java {major}"),
            required_major,
        ));
    }
    Ok(WorkerJvm {
        home,
        major,
        release_digest: canonical::hash_bytes(release.as_bytes()),
        selection: selection.into(),
    })
}

fn unavailable(reason: &str, required_major: u16) -> ClewError {
    ClewError::new(
        ErrorCode::UnsupportedProjectConfiguration,
        format!(
            "WORKER_JDK_UNSUPPORTED: {reason}. Kotlin analysis requires a JDK {required_major} worker runtime. Set {WORKER_JAVA_HOME} to an installed JDK {required_major} home; keep JAVA_HOME configured for the project's Maven/Gradle build. No project compiler target needs to change."
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jdk(root: &Path, major: u16) -> PathBuf {
        let home = root.join(format!("jdk-{major}"));
        std::fs::create_dir_all(home.join("bin")).unwrap();
        std::fs::write(home.join("bin/java"), "fixture").unwrap();
        std::fs::write(home.join("bin/javac"), "fixture").unwrap();
        std::fs::write(
            home.join("release"),
            format!("JAVA_VERSION=\"{major}.0.1\"\n"),
        )
        .unwrap();
        home
    }

    #[test]
    fn worker_jdk_is_separate_from_project_java_home() {
        let root = tempfile::tempdir().unwrap();
        let project = jdk(root.path(), 17);
        let worker = inspect(&jdk(root.path(), 21), "TEST", 21).unwrap();
        let mut command = Command::new("fixture-launcher");
        command.env("JAVA_HOME", &project);
        worker.configure(&mut command);
        let environment = command
            .get_envs()
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(
            environment[std::ffi::OsStr::new("JAVA_HOME")],
            Some(project.as_os_str())
        );
        assert_eq!(
            environment[std::ffi::OsStr::new(WORKER_JAVA_HOME)],
            Some(worker.home.as_os_str())
        );
    }

    #[test]
    fn unsuitable_explicit_jdk_reports_runtime_and_remediation() {
        let root = tempfile::tempdir().unwrap();
        let error = inspect(&jdk(root.path(), 17), "EXPLICIT_WORKER_JDK", 21).unwrap_err();
        assert_eq!(error.code, ErrorCode::UnsupportedProjectConfiguration);
        assert!(error.message.contains("Java 17"));
        assert!(error.message.contains(WORKER_JAVA_HOME));
        assert!(!error.message.contains(root.path().to_str().unwrap()));
    }
}
