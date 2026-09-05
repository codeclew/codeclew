use crate::canonical;
use crate::error::{ClewError, ErrorCode};
use crate::repository_snapshot::isolated_git_command;
use crate::runtime::RuntimeAuthority;
use crate::rust_project_model::RustCompilationSelector;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path};
use std::process::{Command, Stdio};
use walkdir::{DirEntry, WalkDir};

const SCHEMA: &str = "codeclew-repository-diagnostic/1.0";
const MAX_DISCOVERY_ENTRIES: usize = 100_000;
const MAX_COMPILATIONS: usize = 64;
const MAX_CARGO_METADATA_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug, Default)]
struct RepositoryInventory {
    scanned_entries: usize,
    scan_complete: bool,
    selector_limit_exceeded: bool,
    python: bool,
    rust: bool,
    kotlin: bool,
    java: bool,
    typescript: bool,
    javascript: bool,
    gradle: bool,
    maven: bool,
    python_compilations: BTreeSet<String>,
    jvm_compilations: BTreeMap<&'static str, BTreeSet<String>>,
    tsconfig_compilations: BTreeSet<String>,
    unsupported_languages: BTreeSet<&'static str>,
}

#[derive(Debug, Clone)]
struct RepositoryState {
    available: bool,
    git: bool,
    clean: bool,
    target_ref: Option<String>,
    mutation_ref: bool,
}

pub fn diagnose_repository(
    runtime: &RuntimeAuthority,
    matrix: &Value,
    repository: &Path,
) -> Result<Value, ClewError> {
    diagnose_repository_with_source(runtime, matrix, repository, false)
}

pub fn diagnose_repository_with_source(
    runtime: &RuntimeAuthority,
    matrix: &Value,
    repository: &Path,
    committed: bool,
) -> Result<Value, ClewError> {
    let matrix_profiles = matrix
        .get("profiles")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("support matrix profile set is unavailable"))?;
    let support_matrix_digest = canonical::hash(matrix).map_err(internal)?;
    let normalized = repository.canonicalize().ok().filter(|path| path.is_dir());
    let state = repository_state(normalized.as_deref());
    let mut checks = vec![check(
        "repository.available",
        state.available,
        true,
        "SELECT_EXISTING_REPOSITORY",
    )];
    if state.available {
        checks.extend([
            check("repository.git", state.git, true, "SELECT_GIT_REPOSITORY"),
            check(
                "repository.clean",
                state.clean,
                !committed,
                "SELECT_COMMITTED_ANALYSIS_OR_CLEAN_WORKTREE",
            ),
            check(
                "repository.target-ref-local",
                state.target_ref.is_some(),
                true,
                "SELECT_LOCAL_TARGET_REF",
            ),
        ]);
    }

    let mut inventory = normalized
        .as_deref()
        .map(scan_repository)
        .unwrap_or_default();
    if state.available {
        checks.push(check(
            "repository.discovery-complete",
            inventory.scan_complete && !inventory.selector_limit_exceeded,
            true,
            "NARROW_REPOSITORY_DISCOVERY_SCOPE",
        ));
    }

    let mut rust_discovery_error = None;
    if inventory.rust {
        match normalized
            .as_deref()
            .ok_or_else(|| invalid("repository is unavailable"))
            .and_then(discover_rust_compilations)
        {
            Ok(compilations) => {
                inventory
                    .jvm_compilations
                    .insert("rust", compilations.into_iter().collect());
            }
            Err(error) => rust_discovery_error = Some(error.message),
        }
    }

    let common_blockers = common_blockers(&state, &inventory, committed);
    let mut contours = Vec::new();
    if inventory.rust {
        let compilations = inventory
            .jvm_compilations
            .get("rust")
            .cloned()
            .unwrap_or_default();
        let mut blockers = common_blockers.clone();
        if let Some(message) = rust_discovery_error {
            blockers.push(blocker("DISCOVER_CARGO_COMPILATIONS", &message));
        }
        if compilations.is_empty() {
            blockers.push(blocker(
                "SELECT_EXACT_COMPILATION",
                "no exact Cargo target selector was discovered",
            ));
        }
        contours.extend(profile_contours(
            matrix_profiles,
            "rust",
            None,
            compilations,
            blockers,
            &state,
            runtime,
        ));
    }
    if inventory.python {
        contours.extend(profile_contours(
            matrix_profiles,
            "python",
            None,
            inventory.python_compilations.clone(),
            common_blockers.clone(),
            &state,
            runtime,
        ));
    }
    if inventory.gradle || inventory.maven {
        let ambiguous = inventory.gradle && inventory.maven;
        for (build_system, observed) in [
            ("GRADLE_WRAPPER", inventory.gradle),
            ("MAVEN", inventory.maven),
        ] {
            if !observed {
                continue;
            }
            for (language, present) in [("kotlin", inventory.kotlin), ("java", inventory.java)] {
                if !present {
                    continue;
                }
                let mut blockers = common_blockers.clone();
                if ambiguous {
                    blockers.push(blocker(
                        "SELECT_UNAMBIGUOUS_BUILD_SYSTEM",
                        "both Gradle and Maven markers are present at the repository root",
                    ));
                }
                let compilations = inventory
                    .jvm_compilations
                    .get(language)
                    .cloned()
                    .unwrap_or_default();
                if compilations.is_empty() {
                    blockers.push(blocker(
                        "SELECT_EXACT_COMPILATION",
                        "no JVM source-set selector was discovered from repository paths",
                    ));
                }
                add_build_tool_blockers(&mut blockers, build_system, normalized.as_deref());
                contours.extend(profile_contours(
                    matrix_profiles,
                    language,
                    Some(build_system),
                    compilations,
                    blockers,
                    &state,
                    runtime,
                ));
            }
        }
    } else {
        for (language, present) in [("kotlin", inventory.kotlin), ("java", inventory.java)] {
            if present {
                let mut blockers = common_blockers.clone();
                blockers.push(blocker(
                    "SELECT_SUPPORTED_BUILD_SYSTEM",
                    "source files were detected without a root Gradle wrapper or Maven project",
                ));
                contours.extend(profile_contours(
                    matrix_profiles,
                    language,
                    None,
                    BTreeSet::new(),
                    blockers,
                    &state,
                    runtime,
                ));
            }
        }
    }
    for (language, present) in [
        ("typescript", inventory.typescript),
        ("javascript", inventory.javascript),
    ] {
        if !present {
            continue;
        }
        let mut blockers = common_blockers.clone();
        if inventory.tsconfig_compilations.is_empty() {
            blockers.push(blocker(
                "SELECT_TSCONFIG",
                "source files were detected without a bounded tsconfig candidate",
            ));
        }
        if !executable_available("node") {
            blockers.push(blocker("INSTALL_NODE", "Node.js is unavailable on PATH"));
        } else if normalized
            .as_deref()
            .is_some_and(|repository| !typescript_5_available(repository))
        {
            blockers.push(blocker(
                "INSTALL_TYPESCRIPT_5",
                "project-resolvable TypeScript 5.x is unavailable",
            ));
        }
        contours.extend(profile_contours(
            matrix_profiles,
            language,
            Some("TSCONFIG"),
            inventory.tsconfig_compilations.clone(),
            blockers,
            &state,
            runtime,
        ));
    }

    contours.sort_by(|left, right| {
        left["language"]
            .as_str()
            .cmp(&right["language"].as_str())
            .then_with(|| left["profileId"].as_str().cmp(&right["profileId"].as_str()))
    });
    if committed {
        for contour in &mut contours {
            contour["supportedOperations"] = json!(["ANALYSIS"]);
            contour["sourceArguments"] = json!(["--committed"]);
        }
    }
    let ready_count = contours
        .iter()
        .filter(|contour| contour["status"] == "READY_FOR_TASK_DOCTOR")
        .count();
    let status = if !state.available {
        "ACTION_REQUIRED"
    } else if contours.is_empty() {
        "UNSUPPORTED"
    } else if ready_count == contours.len() && inventory.unsupported_languages.is_empty() {
        "READY_FOR_TASK_DOCTOR"
    } else if ready_count > 0 {
        "PARTIALLY_READY"
    } else {
        "ACTION_REQUIRED"
    };
    let next_action = if ready_count > 0 {
        "RUN_TASK_DOCTOR"
    } else {
        contours
            .iter()
            .flat_map(|contour| contour["blockers"].as_array().into_iter().flatten())
            .find_map(|row| row["remediationId"].as_str())
            .or_else(|| {
                checks
                    .iter()
                    .find(|row| row["status"] == "ACTION_REQUIRED")
                    .and_then(|row| row["remediationId"].as_str())
            })
            .unwrap_or("SELECT_SUPPORTED_REPOSITORY")
    };

    Ok(json!({
        "schema":SCHEMA,
        "status":status,
        "nextAction":next_action,
        "sourceSelection":{
            "kind":"COMMITTED_HEAD",
            "uncommittedChangesIncluded":false,
            "dirtyWorktreeAllowed":committed,
        },
        "nextActions":if state.git && !state.clean && !committed {
            json!({
                "reason":"LOCAL_EDITS_PRESENT",
                "message":"For read-only analysis of committed HEAD, repeat doctor repository with --committed and pass --committed to context open or nav query. Local edits are excluded from the snapshot. To analyze those edits, commit them first; do not discard or stash work just to run Codeclew.",
                "argumentsToAdd":["--committed"],
                "operation":"ANALYSIS",
                "uncommittedChangesIncluded":false,
            })
        } else { Value::Null },
        "runtimeMode":runtime.mode,
        "runtimeKey":runtime.runtime_key,
        "runtimeManifestDigest":runtime.manifest_digest,
        "supportMatrixDigest":support_matrix_digest,
        "repository":{
            "clean":state.clean,
            "git":state.git,
            "mutationRef":state.mutation_ref,
            "targetRef":state.target_ref,
        },
        "checks":checks,
        "contours":contours,
        "unsupportedLanguages":inventory.unsupported_languages,
        "discovery":{
            "complete":inventory.scan_complete && !inventory.selector_limit_exceeded,
            "compilationLimit":MAX_COMPILATIONS,
            "compilationLimitExceeded":inventory.selector_limit_exceeded,
            "entryLimit":MAX_DISCOVERY_ENTRIES,
            "scannedEntries":inventory.scanned_entries,
        },
        "privacyAssertions":{
            "containsAbsolutePaths":false,
            "containsRepositoryIdentity":true,
            "containsSource":false,
        },
    }))
}

fn repository_state(repository: Option<&Path>) -> RepositoryState {
    let Some(repository) = repository else {
        return RepositoryState {
            available: false,
            git: false,
            clean: false,
            target_ref: None,
            mutation_ref: false,
        };
    };
    let git = isolated_git(repository, &["rev-parse", "--is-inside-work-tree"])
        .is_some_and(|value| value == "true");
    let clean = git
        && isolated_git(
            repository,
            &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
        )
        .is_some_and(|value| value.is_empty());
    let branch = git
        .then(|| isolated_git(repository, &["symbolic-ref", "-q", "HEAD"]))
        .flatten()
        .filter(|value| value.starts_with("refs/heads/"));
    let target_ref = branch.or_else(|| unique_tag_at_head(repository));
    RepositoryState {
        available: true,
        git,
        clean,
        mutation_ref: target_ref
            .as_deref()
            .is_some_and(|value| value.starts_with("refs/heads/")),
        target_ref,
    }
}

fn unique_tag_at_head(repository: &Path) -> Option<String> {
    let output = isolated_git(
        repository,
        &[
            "for-each-ref",
            "--points-at=HEAD",
            "--format=%(refname)",
            "refs/tags",
        ],
    )?;
    let tags = output
        .lines()
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    (tags.len() == 1).then(|| tags[0].to_owned())
}

fn common_blockers(
    state: &RepositoryState,
    inventory: &RepositoryInventory,
    committed: bool,
) -> Vec<Value> {
    let mut blockers = Vec::new();
    if !state.git {
        blockers.push(blocker(
            "SELECT_GIT_REPOSITORY",
            "the selected directory is not a Git worktree",
        ));
    }
    if state.git && !state.clean && !committed {
        blockers.push(blocker(
            "SELECT_COMMITTED_ANALYSIS_OR_CLEAN_WORKTREE",
            "Local edits are present. Use --committed for read-only analysis of committed HEAD, excluding those edits; mutation requires a clean worktree.",
        ));
    }
    if state.git && state.target_ref.is_none() {
        blockers.push(blocker(
            "SELECT_LOCAL_TARGET_REF",
            "HEAD is not bound to one local branch or exact local tag",
        ));
    }
    if !inventory.scan_complete {
        blockers.push(blocker(
            "NARROW_REPOSITORY_DISCOVERY_SCOPE",
            "repository discovery exceeded its bounded entry limit or hit an unreadable path",
        ));
    }
    if inventory.selector_limit_exceeded {
        blockers.push(blocker(
            "NARROW_REPOSITORY_DISCOVERY_SCOPE",
            "repository discovery exceeded the exact compilation selector limit",
        ));
    }
    blockers
}

fn profile_contours(
    profiles: &[Value],
    language: &str,
    build_system: Option<&str>,
    compilations: BTreeSet<String>,
    blockers: Vec<Value>,
    state: &RepositoryState,
    runtime: &RuntimeAuthority,
) -> Vec<Value> {
    profiles
        .iter()
        .filter(|profile| profile["language"].as_str() == Some(language))
        .filter(|profile| {
            build_system.is_none()
                || profile["buildSystem"].as_str().is_none()
                || profile["buildSystem"].as_str() == build_system
        })
        .map(|profile| {
            let mut blockers = blockers.clone();
            if language == "kotlin" {
                let compiler_version =
                    if profile["semanticEngine"].as_str() == Some("kotlin-engine-2.4.10") {
                        Some("2.4.10")
                    } else {
                        profile["compilerVersion"].as_str()
                    };
                if !compiler_version.is_some_and(|expected| {
                    runtime
                        .workers
                        .values()
                        .any(|worker| worker.compiler_version == expected)
                }) {
                    blockers.push(blocker(
                        "INSTALL_QUALIFIED_RUNTIME",
                        "the active runtime has no worker for this Kotlin profile",
                    ));
                }
            }
            let mut operations = vec!["ANALYSIS"];
            let mut mutation_blockers = Vec::new();
            if profile["mutation"].as_bool() == Some(true) {
                if state.mutation_ref {
                    operations.push("MUTATION");
                } else {
                    mutation_blockers.push(blocker(
                        "SELECT_LOCAL_BRANCH_REF",
                        "mutation requires a checked-out local branch",
                    ));
                }
            }
            json!({
                "analysisAuthority":profile["analysisAuthority"],
                "blockers":blockers,
                "buildSystem":profile.get("buildSystem").cloned().unwrap_or(Value::Null),
                "compilations":compilations,
                "language":language.to_ascii_uppercase(),
                "mutationBlockers":mutation_blockers,
                "supportedOperations":operations,
                "profileId":profile["profileId"],
                "profileStatus":profile["status"],
                "status":if blockers.is_empty() && !compilations.is_empty() {
                    "READY_FOR_TASK_DOCTOR"
                } else {
                    "ACTION_REQUIRED"
                },
            })
        })
        .collect()
}

fn add_build_tool_blockers(
    blockers: &mut Vec<Value>,
    build_system: &str,
    repository: Option<&Path>,
) {
    let Some(repository) = repository else {
        return;
    };
    match build_system {
        "GRADLE_WRAPPER" if !executable_file(&repository.join("gradlew")) => {
            blockers.push(blocker(
                "INSTALL_PROJECT_WRAPPER",
                "the Gradle wrapper is missing or not executable; restore the project wrapper, then run chmod +x ./gradlew and ./gradlew --version from the repository root",
            ))
        }
        "MAVEN" if !executable_file(&repository.join("mvnw")) && !executable_available("mvn") => {
            blockers.push(blocker(
                "INSTALL_PROJECT_LAUNCHER",
                "no executable Maven launcher was found in this process environment; from the same terminal or agent environment run command -v mvn and mvn --version, then expose Maven on PATH or restore an executable ./mvnw and rerun doctor repository",
            ));
        }
        _ => {}
    }
    if !executable_available("java") {
        blockers.push(blocker("INSTALL_JDK_21", "Java is unavailable on PATH"));
    }
}

fn scan_repository(repository: &Path) -> RepositoryInventory {
    let mut inventory = RepositoryInventory {
        scan_complete: true,
        ..RepositoryInventory::default()
    };
    inventory.gradle = regular_file(&repository.join("gradlew"))
        && (regular_file(&repository.join("settings.gradle"))
            || regular_file(&repository.join("settings.gradle.kts")));
    inventory.maven = regular_file(&repository.join("pom.xml"));
    inventory.rust = regular_file(&repository.join("Cargo.toml"))
        && regular_file(&repository.join("Cargo.lock"));

    for result in WalkDir::new(repository)
        .follow_links(false)
        .into_iter()
        .filter_entry(included_entry)
    {
        if inventory.scanned_entries >= MAX_DISCOVERY_ENTRIES {
            inventory.scan_complete = false;
            break;
        }
        inventory.scanned_entries += 1;
        let Ok(entry) = result else {
            inventory.scan_complete = false;
            continue;
        };
        if !entry.file_type().is_file() || entry.file_type().is_symlink() {
            continue;
        }
        let Some(relative) = safe_relative(repository, entry.path()) else {
            inventory.scan_complete = false;
            continue;
        };
        let file_name = entry.file_name().to_string_lossy();
        if file_name.starts_with("tsconfig")
            && file_name.ends_with(".json")
            && !insert_bounded(
                &mut inventory.tsconfig_compilations,
                format!("tsconfig:{relative}"),
            )
        {
            inventory.selector_limit_exceeded = true;
        }
        match entry.path().extension().and_then(|value| value.to_str()) {
            Some("rs") => inventory.rust = true,
            Some("py") => inventory.python = true,
            Some("kt") => {
                inventory.kotlin = true;
                add_jvm_compilation(&mut inventory, "kotlin", &relative);
            }
            Some("kts")
                if !matches!(
                    file_name.as_ref(),
                    "build.gradle.kts" | "settings.gradle.kts"
                ) =>
            {
                inventory.kotlin = true;
                add_jvm_compilation(&mut inventory, "kotlin", &relative);
            }
            Some("java") => {
                inventory.java = true;
                add_jvm_compilation(&mut inventory, "java", &relative);
            }
            Some("ts" | "tsx" | "mts" | "cts") => inventory.typescript = true,
            Some("js" | "jsx" | "mjs" | "cjs") => inventory.javascript = true,
            Some(extension) => {
                if let Some(language) = unsupported_language(extension) {
                    inventory.unsupported_languages.insert(language);
                }
            }
            None => {}
        }
    }
    if inventory.python {
        inventory.python_compilations.insert("python:.#.".into());
    }
    inventory
}

fn add_jvm_compilation(
    inventory: &mut RepositoryInventory,
    language: &'static str,
    relative: &str,
) {
    let components = relative.split('/').collect::<Vec<_>>();
    for index in 0..components.len().saturating_sub(2) {
        if components[index] == "src" && matches!(components[index + 1], "main" | "test") {
            let project = if index == 0 {
                ":".into()
            } else {
                format!(":{}", components[..index].join(":"))
            };
            if !insert_bounded(
                inventory.jvm_compilations.entry(language).or_default(),
                format!("{project}/{}", components[index + 1]),
            ) {
                inventory.selector_limit_exceeded = true;
            }
            break;
        }
    }
}

fn insert_bounded(values: &mut BTreeSet<String>, value: String) -> bool {
    values.contains(&value) || (values.len() < MAX_COMPILATIONS && values.insert(value))
}

fn discover_rust_compilations(repository: &Path) -> Result<Vec<String>, ClewError> {
    if !executable_available("cargo") || !executable_available("rustc") {
        return Err(ClewError::new(
            ErrorCode::UnsupportedProjectConfiguration,
            "Cargo and rustc must both be available on PATH",
        ));
    }
    let output = Command::new("cargo")
        .args([
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--manifest-path",
        ])
        .arg(repository.join("Cargo.toml"))
        .current_dir(repository)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .map_err(|_| unsupported("Cargo metadata could not start"))?;
    if !output.status.success()
        || output.stdout.is_empty()
        || output.stdout.len() > MAX_CARGO_METADATA_BYTES
    {
        return Err(unsupported(
            "Cargo metadata did not produce a bounded project model",
        ));
    }
    let metadata: Value = serde_json::from_slice(&output.stdout)
        .map_err(|_| unsupported("Cargo metadata response is invalid"))?;
    let workspace_root = metadata["workspace_root"]
        .as_str()
        .ok_or_else(|| unsupported("Cargo workspace root is unavailable"))?;
    let normalized_workspace = Path::new(workspace_root)
        .canonicalize()
        .map_err(|_| unsupported("Cargo workspace root cannot be resolved"))?;
    if normalized_workspace != repository {
        return Err(unsupported(
            "Cargo workspace root differs from the selected repository root",
        ));
    }
    let packages = metadata["packages"]
        .as_array()
        .ok_or_else(|| unsupported("Cargo metadata has no package set"))?;
    let mut selectors = BTreeSet::new();
    for package in packages {
        let package_name = package["name"]
            .as_str()
            .ok_or_else(|| unsupported("Cargo package name is invalid"))?;
        let manifest = package["manifest_path"]
            .as_str()
            .and_then(|path| safe_relative(repository, Path::new(path)))
            .ok_or_else(|| unsupported("Cargo package manifest path is unsafe"))?;
        let targets = package["targets"]
            .as_array()
            .ok_or_else(|| unsupported("Cargo package has no target set"))?;
        for target in targets {
            let target_name = target["name"]
                .as_str()
                .ok_or_else(|| unsupported("Cargo target name is invalid"))?;
            let kinds = target["kind"]
                .as_array()
                .ok_or_else(|| unsupported("Cargo target kind is invalid"))?;
            for kind in kinds {
                let selector = RustCompilationSelector {
                    manifest: manifest.clone(),
                    package: package_name.into(),
                    target_kind: kind
                        .as_str()
                        .ok_or_else(|| unsupported("Cargo target kind is invalid"))?
                        .into(),
                    target_name: target_name.into(),
                }
                .canonical();
                RustCompilationSelector::parse(&selector)?;
                selectors.insert(selector);
                if selectors.len() > MAX_COMPILATIONS {
                    return Err(unsupported(
                        "Cargo target count exceeds the session compilation limit",
                    ));
                }
            }
        }
    }
    Ok(selectors.into_iter().collect())
}

fn included_entry(entry: &DirEntry) -> bool {
    if entry.depth() == 0 || !entry.file_type().is_dir() {
        return true;
    }
    !matches!(
        entry.file_name().to_str(),
        Some(
            ".git"
                | ".gradle"
                | ".idea"
                | ".venv"
                | ".vscode"
                | "build"
                | "dist"
                | "node_modules"
                | "out"
                | "target"
                | "venv"
        )
    )
}

fn unsupported_language(extension: &str) -> Option<&'static str> {
    match extension {
        "c" | "h" => Some("C"),
        "cc" | "cpp" | "cxx" | "hpp" => Some("C++"),
        "cs" => Some("C_SHARP"),
        "go" => Some("GO"),
        "php" => Some("PHP"),
        "rb" => Some("RUBY"),
        "scala" => Some("SCALA"),
        "swift" => Some("SWIFT"),
        _ => None,
    }
}

fn check(id: &str, passed: bool, required: bool, remediation: &str) -> Value {
    json!({
        "checkId":id,
        "remediationId":if passed { Value::Null } else { Value::String(remediation.into()) },
        "required":required,
        "status":if passed { "PASS" } else { "ACTION_REQUIRED" },
    })
}

fn blocker(remediation: &str, message: &str) -> Value {
    json!({
        "message":message,
        "remediationId":remediation,
    })
}

fn isolated_git(repository: &Path, arguments: &[&str]) -> Option<String> {
    let mut command = isolated_git_command(repository);
    let output = command
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() || output.stdout.len() > 1024 * 1024 {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()
        .map(|value| value.trim().to_owned())
}

fn typescript_5_available(repository: &Path) -> bool {
    let output = Command::new("node")
        .args([
            "-e",
            "const p=require('typescript/package.json');process.stdout.write(p.version)",
        ])
        .current_dir(repository)
        .env_remove("NODE_OPTIONS")
        .env_remove("NODE_PATH")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output();
    output.is_ok_and(|output| {
        output.status.success()
            && output.stdout.len() <= 64
            && std::str::from_utf8(&output.stdout).is_ok_and(|version| version.starts_with("5."))
    })
}

fn executable_available(name: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|directory| executable_on_search_path(&directory.join(name)))
}

// Host package managers commonly expose tools through symlinks. Repository
// wrappers retain their separate, non-symlink authority check below.
fn executable_on_search_path(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|metadata| metadata.is_file() && executable_permissions(&metadata))
}

fn executable_file(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|metadata| {
        metadata.is_file()
            && !metadata.file_type().is_symlink()
            && executable_permissions(&metadata)
    })
}

#[cfg(unix)]
fn executable_permissions(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn executable_permissions(_: &fs::Metadata) -> bool {
    true
}

fn regular_file(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
}

fn safe_relative(repository: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(repository).ok()?;
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return None;
    }
    let value = relative.to_str()?.replace(std::path::MAIN_SEPARATOR, "/");
    (!value.is_empty() && value.len() <= 1024 && !value.contains(['\0', '#'])).then_some(value)
}

fn invalid(message: &str) -> ClewError {
    ClewError::new(ErrorCode::InvalidInput, message)
}

fn unsupported(message: &str) -> ClewError {
    ClewError::new(ErrorCode::UnsupportedProjectConfiguration, message)
}

fn internal(error: impl std::fmt::Display) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_COMPILATIONS, RepositoryInventory, add_jvm_compilation, diagnose_repository,
        diagnose_repository_with_source, safe_relative, scan_repository,
    };
    use crate::operations::support_matrix;
    use crate::runtime::{RuntimeAuthority, RuntimeMode};
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::Path;
    use std::process::Command;

    #[cfg(unix)]
    #[test]
    fn path_tools_accept_package_manager_symlinks_but_wrappers_do_not() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let root = tempfile::tempdir().unwrap();
        let tool = root.path().join("maven-launcher");
        let link = root.path().join("mvn");
        fs::write(&tool, "#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(&tool, fs::Permissions::from_mode(0o755)).unwrap();
        symlink(&tool, &link).unwrap();
        assert!(super::executable_on_search_path(&link));
        assert!(!super::executable_file(&link));
        fs::set_permissions(&tool, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(!super::executable_on_search_path(&link));
        fs::remove_file(&tool).unwrap();
        assert!(!super::executable_on_search_path(&link));
    }

    #[test]
    fn scan_is_bounded_and_discovers_mixed_language_candidates() {
        let repository = tempfile::tempdir().unwrap();
        fs::create_dir_all(repository.path().join("app/src/main/kotlin")).unwrap();
        fs::create_dir_all(repository.path().join("web/src")).unwrap();
        fs::create_dir_all(repository.path().join("scripts")).unwrap();
        fs::write(
            repository.path().join("app/src/main/kotlin/App.kt"),
            "class App",
        )
        .unwrap();
        fs::write(
            repository.path().join("web/src/app.ts"),
            "export const app = 1",
        )
        .unwrap();
        fs::write(repository.path().join("web/tsconfig.json"), "{}").unwrap();
        fs::write(repository.path().join("scripts/check.py"), "pass").unwrap();
        fs::write(repository.path().join("unsupported.go"), "package main").unwrap();

        let inventory = scan_repository(repository.path());

        assert!(inventory.scan_complete);
        assert!(inventory.kotlin);
        assert!(inventory.typescript);
        assert!(inventory.python);
        assert_eq!(
            inventory.jvm_compilations["kotlin"],
            [":app/main".to_owned()].into_iter().collect()
        );
        assert_eq!(
            inventory.tsconfig_compilations,
            ["tsconfig:web/tsconfig.json".to_owned()]
                .into_iter()
                .collect()
        );
        assert_eq!(
            inventory.python_compilations,
            ["python:.#.".to_owned()].into_iter().collect()
        );
        assert!(inventory.unsupported_languages.contains("GO"));
    }

    #[test]
    fn jvm_selector_uses_source_set_and_module_path() {
        let mut inventory = RepositoryInventory::default();
        add_jvm_compilation(&mut inventory, "java", "src/test/java/RootTest.java");
        add_jvm_compilation(
            &mut inventory,
            "java",
            "services/api/src/main/java/Api.java",
        );
        assert_eq!(
            inventory.jvm_compilations["java"],
            [":/test".to_owned(), ":services:api/main".to_owned()]
                .into_iter()
                .collect()
        );
    }

    #[test]
    fn jvm_selector_discovery_is_bounded() {
        let mut inventory = RepositoryInventory::default();
        for index in 0..=MAX_COMPILATIONS {
            add_jvm_compilation(
                &mut inventory,
                "java",
                &format!("module{index}/src/main/java/App.java"),
            );
        }
        assert_eq!(inventory.jvm_compilations["java"].len(), MAX_COMPILATIONS);
        assert!(inventory.selector_limit_exceeded);
    }

    #[test]
    fn safe_relative_rejects_the_repository_root() {
        let repository = tempfile::tempdir().unwrap();
        assert!(safe_relative(repository.path(), repository.path()).is_none());
    }

    #[test]
    fn diagnostic_reports_a_clean_python_repository_as_ready() {
        let repository = tempfile::tempdir().unwrap();
        fs::write(repository.path().join("app.py"), "value = 1\n").unwrap();
        git(repository.path(), &["init"]);
        git(repository.path(), &["add", "app.py"]);
        git(
            repository.path(),
            &[
                "-c",
                "user.name=Codeclew Test",
                "-c",
                "user.email=codeclew@example.invalid",
                "commit",
                "-m",
                "fixture",
            ],
        );

        let value =
            diagnose_repository(&runtime(), &support_matrix().unwrap(), repository.path()).unwrap();

        assert_eq!(value["schema"], "codeclew-repository-diagnostic/1.0");
        assert_eq!(value["status"], "READY_FOR_TASK_DOCTOR");
        assert_eq!(value["nextAction"], "RUN_TASK_DOCTOR");
        assert_eq!(value["contours"][0]["language"], "PYTHON");
        assert_eq!(value["contours"][0]["compilations"][0], "python:.#.");
        fs::write(repository.path().join("app.py"), "local = 2\n").unwrap();
        let dirty =
            diagnose_repository(&runtime(), &support_matrix().unwrap(), repository.path()).unwrap();
        assert_eq!(dirty["status"], "ACTION_REQUIRED");
        assert_eq!(
            dirty["nextActions"]["argumentsToAdd"],
            json!(["--committed"])
        );
        let committed = diagnose_repository_with_source(
            &runtime(),
            &support_matrix().unwrap(),
            repository.path(),
            true,
        )
        .unwrap();
        assert_eq!(committed["status"], "READY_FOR_TASK_DOCTOR");
        assert_eq!(committed["repository"]["clean"], false);
        assert_eq!(
            committed["sourceSelection"]["uncommittedChangesIncluded"],
            false
        );
        assert_eq!(
            committed["contours"][0]["supportedOperations"],
            json!(["ANALYSIS"])
        );
        assert_eq!(value["privacyAssertions"]["containsAbsolutePaths"], false);
        assert_eq!(
            value["privacyAssertions"]["containsRepositoryIdentity"],
            true
        );
        assert!(
            !value
                .to_string()
                .contains(repository.path().to_str().unwrap())
        );
    }

    #[test]
    fn diagnostic_reports_supported_and_unsupported_sources_as_partially_ready() {
        let repository = tempfile::tempdir().unwrap();
        fs::write(repository.path().join("app.py"), "value = 1\n").unwrap();
        fs::write(repository.path().join("main.go"), "package main\n").unwrap();
        git(repository.path(), &["init"]);
        git(repository.path(), &["add", "."]);
        git(
            repository.path(),
            &[
                "-c",
                "user.name=Codeclew Test",
                "-c",
                "user.email=codeclew@example.invalid",
                "commit",
                "-m",
                "fixture",
            ],
        );

        let value =
            diagnose_repository(&runtime(), &support_matrix().unwrap(), repository.path()).unwrap();

        assert_eq!(value["status"], "PARTIALLY_READY");
        assert_eq!(value["nextAction"], "RUN_TASK_DOCTOR");
        assert_eq!(value["unsupportedLanguages"][0], "GO");
        assert_eq!(value["contours"][0]["status"], "READY_FOR_TASK_DOCTOR");
    }

    fn runtime() -> RuntimeAuthority {
        RuntimeAuthority {
            schema: "codeclew-runtime-capsule/4.0".into(),
            runtime_key: format!("sha256:{}", "1".repeat(64)),
            mode: RuntimeMode::Release,
            manifest_digest: format!("sha256:{}", "2".repeat(64)),
            components: BTreeMap::new(),
            artifacts: BTreeMap::new(),
            workers: BTreeMap::new(),
            root: std::path::PathBuf::new(),
        }
    }

    fn git(repository: &Path, arguments: &[&str]) {
        let output = Command::new("git")
            .args(arguments)
            .current_dir(repository)
            .output()
            .unwrap();
        assert!(output.status.success(), "git {arguments:?} failed");
    }
}
